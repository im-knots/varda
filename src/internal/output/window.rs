//! Windowed and display outputs: an OS window, floating or fullscreen on a
//! monitor.
//!
//! A window can only be created on the event loop, so a new window sink asks
//! for one ([`OutputSinkInstance::window_request`]) and draws nothing until
//! the runner has created it and handed it over
//! ([`OutputSinkInstance::attach_window`]). Headless runs have no event loop,
//! and report both types unavailable. See /spec/output-sink-providers.md
//! Decisions 6 and 9.

use super::{
    ControlSpec, ControlValue, FramePath, LibraryEntry, LibrarySection, OutputSinkInstance,
    OutputSinkProvider, ParamEffect, Presentation, SinkConfig, SinkEnv, SinkQuery, WidgetHint,
};
use crate::engine::value::render::{
    PresentationDepth, PresentationRequest, ResolvedPresentation, Unassigned,
};
use crate::renderer::context::{GpuContext, select_surface_presentation};
use crate::source::ControlError;
use anyhow::{Context, Result};
use std::sync::{Arc, LazyLock};
use winit::window::Window;

pub const WINDOWED: &str = "windowed";
pub const DISPLAY: &str = "display";

/// Monitors the event loop last reported, by name. Refreshed by the runner
/// through the window host; read by the display type's library.
#[derive(Default)]
pub struct Monitors {
    pub list: Vec<(String, winit::monitor::MonitorHandle)>,
}

impl Monitors {
    fn handle(&self, name: &str) -> Option<winit::monitor::MonitorHandle> {
        self.list
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, h)| h.clone())
    }
}

static DISPLAY_PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| {
    vec![
        ControlSpec::text("monitor", "Monitor")
            .routed("monitor")
            .in_widget(WidgetHint::Monitor),
    ]
});

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Config {
    /// The monitor a display output fills. Absent for a floating window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// Window position, restored on the next launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<[i32; 2]>,
    /// Window size in physical pixels, restored on the next launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<[u32; 2]>,
}

/// A floating output window.
pub struct WindowedProvider;

impl OutputSinkProvider for WindowedProvider {
    fn id(&self) -> &'static str {
        WINDOWED
    }

    fn label(&self) -> &'static str {
        "Window"
    }

    fn icon(&self) -> &'static str {
        "🗔"
    }

    fn create(
        &mut self,
        config: &SinkConfig,
        _env: &mut SinkEnv,
    ) -> Result<Box<dyn OutputSinkInstance>> {
        let mut config: Config = crate::source::decode_config(config)?;
        config.name = None;
        Ok(Box::new(WindowSink::new(config, false)))
    }
}

/// A window filling one monitor.
pub struct DisplayProvider;

impl OutputSinkProvider for DisplayProvider {
    fn id(&self) -> &'static str {
        DISPLAY
    }

    fn label(&self) -> &'static str {
        "Display"
    }

    fn icon(&self) -> &'static str {
        "🖥"
    }

    fn params(&self) -> &'static [ControlSpec] {
        &DISPLAY_PARAMS
    }

    /// Every monitor, as a ready-made display config.
    fn library(&self, query: &SinkQuery) -> LibrarySection {
        let entries = query
            .services
            .get::<Monitors>()
            .map(|monitors| {
                monitors
                    .list
                    .iter()
                    .map(|(name, _)| {
                        LibraryEntry::new(name.clone(), SinkConfig::new(DISPLAY).with("name", name))
                    })
                    .collect()
            })
            .unwrap_or_default();
        LibrarySection {
            entries,
            rescan: true,
            ..LibrarySection::default()
        }
    }

    /// Monitors are listed by the event loop, which refreshes them every time
    /// it creates windows, so a rescan has nothing more to do than wait for
    /// the next pass.
    fn library_action(&mut self, action: &str, _env: &mut SinkEnv) -> Result<()> {
        anyhow::ensure!(action == "rescan", "displays have no action '{action}'");
        Ok(())
    }

    fn create(
        &mut self,
        config: &SinkConfig,
        _env: &mut SinkEnv,
    ) -> Result<Box<dyn OutputSinkInstance>> {
        let mut config: Config = crate::source::decode_config(config)?;
        config.name = config.name.filter(|name| !name.is_empty());
        Ok(Box::new(WindowSink::new(config, true)))
    }
}

/// The OS window and its swap chain, once the event loop has created them.
struct Bound {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    capabilities: wgpu::SurfaceCapabilities,
}

/// A window or display output's sink.
pub struct WindowSink {
    config: Config,
    /// A display fills a monitor. One with no monitor chosen yet opens no
    /// window, so a new display never takes over a screen by itself.
    display: bool,
    bound: Option<Bound>,
    /// Set when a display's monitor was missing and it opened floating.
    notice: Option<String>,
}

impl WindowSink {
    fn new(config: Config, display: bool) -> Self {
        Self {
            config,
            display,
            bound: None,
            notice: None,
        }
    }

    fn fullscreen(&self, monitors: Option<&Monitors>) -> Option<winit::window::Fullscreen> {
        let name = self.config.name.as_deref()?;
        let handle = monitors.and_then(|m| m.handle(name));
        Some(winit::window::Fullscreen::Borderless(handle))
    }

    /// Put the window on its monitor, or float it when there is none. A
    /// display whose monitor is gone opens floating and says so.
    fn place(&mut self, monitors: Option<&Monitors>) {
        let Some(bound) = &self.bound else {
            return;
        };
        match (&self.config.name, self.fullscreen(monitors)) {
            (Some(name), Some(winit::window::Fullscreen::Borderless(None))) => {
                bound.window.set_fullscreen(None);
                self.notice = Some(format!(
                    "Monitor '{name}' not connected — output opened as a window"
                ));
            }
            (_, fullscreen) => bound.window.set_fullscreen(fullscreen),
        }
    }

    fn reconfigure(&mut self, gpu: &GpuContext) {
        if let Some(bound) = &self.bound {
            bound.surface.configure(&gpu.device, &bound.surface_config);
        }
    }
}

impl OutputSinkInstance for WindowSink {
    fn sink_type(&self) -> &str {
        if self.display { DISPLAY } else { WINDOWED }
    }

    fn label(&self) -> String {
        self.config.name.clone().unwrap_or_else(|| {
            if self.display {
                "No monitor chosen".into()
            } else {
                "Floating window".into()
            }
        })
    }

    fn config(&self) -> SinkConfig {
        let mut config = self.config.clone();
        if let Some(bound) = &self.bound {
            let size = bound.window.inner_size();
            config.size = Some([size.width, size.height]);
            if let Ok(pos) = bound.window.outer_position() {
                config.position = Some([pos.x, pos.y]);
            }
        }
        crate::source::encode_config(self.sink_type(), &config)
    }

    /// Moving between floating and a monitor, or to another monitor, keeps the
    /// window rather than closing and reopening it.
    fn patch(&mut self, config: &SinkConfig, services: &crate::source::Services) -> bool {
        if !matches!(config.type_id(), WINDOWED | DISPLAY) {
            return false;
        }
        let Ok(next) = crate::source::decode_config::<Config>(config) else {
            return false;
        };
        self.display = config.type_id() == DISPLAY;
        self.config.name = next.name.filter(|name| self.display && !name.is_empty());
        self.notice = None;
        self.place(services.get::<Monitors>());
        true
    }

    fn frame_path(&self) -> FramePath {
        FramePath::Present
    }

    fn default_unassigned(&self) -> Unassigned {
        Unassigned::Stage
    }

    fn size(&self, render: (u32, u32)) -> (u32, u32) {
        self.bound.as_ref().map_or(render, |b| {
            let size = b.window.inner_size();
            (size.width, size.height)
        })
    }

    fn is_ready(&self) -> bool {
        self.bound.is_some()
    }

    fn startable(&self) -> bool {
        false
    }

    fn configure(
        &mut self,
        gpu: &GpuContext,
        _query: &SinkQuery,
        request: PresentationRequest,
    ) -> Result<Presentation> {
        let Some(bound) = &mut self.bound else {
            // Until the window exists, promise what every surface can show.
            let resolved = crate::delivery::presentation::resolve_eight_bit_sdr(
                request,
                "the output window is not open yet",
            );
            return Ok(Presentation {
                modes: crate::delivery::presentation::modes_for(|r| {
                    crate::delivery::presentation::resolve_eight_bit_sdr(
                        r,
                        "the output window is not open yet",
                    )
                }),
                resolved,
            });
        };
        let selection = select_surface_presentation(request, &bound.capabilities)?;
        if bound.surface_config.format != selection.format
            || bound.surface_config.color_space != selection.color_space
        {
            bound.surface_config.format = selection.format;
            bound.surface_config.color_space = selection.color_space;
            bound.surface.configure(&gpu.device, &bound.surface_config);
        }
        Ok(Presentation {
            resolved: selection.resolved,
            modes: selection.mode_availability,
        })
    }

    fn target_format(&self, _resolved: &ResolvedPresentation) -> wgpu::TextureFormat {
        self.bound
            .as_ref()
            .map_or(wgpu::TextureFormat::Bgra8UnormSrgb, |b| {
                b.surface_config.format
            })
    }

    /// Ten-bit output composes in float; eight-bit in the surface's own format,
    /// as a window always has.
    fn intermediate_format(&self, resolved: &ResolvedPresentation) -> wgpu::TextureFormat {
        if resolved.resolved == PresentationDepth::Sdr10 {
            crate::renderer::context::COLOR_PATH_FORMAT
        } else {
            self.target_format(resolved)
        }
    }

    fn acquire(&mut self, gpu: &GpuContext) -> Option<wgpu::SurfaceTexture> {
        let label = self.label();
        let bound = self.bound.as_ref()?;
        match bound.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => Some(frame),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                log::warn!("Output '{label}': surface suboptimal, will reconfigure");
                Some(frame)
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                log::warn!("Output '{label}': surface outdated, reconfiguring");
                self.reconfigure(gpu);
                match self.bound.as_ref()?.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
                    other => {
                        log::error!(
                            "Output '{label}': no surface texture after reconfiguring: {other:?}"
                        );
                        None
                    }
                }
            }
            other => {
                log::debug!("Output '{label}': surface unavailable: {other:?}");
                None
            }
        }
    }

    fn present(&mut self, gpu: &GpuContext, frame: wgpu::SurfaceTexture) {
        gpu.queue.present(frame);
        if let Some(bound) = &self.bound {
            bound.window.request_redraw();
        }
    }

    fn schema(&self) -> &'static [ControlSpec] {
        if self.display { &DISPLAY_PARAMS } else { &[] }
    }

    fn param(&self, name: &str) -> Option<ControlValue> {
        (name == "monitor")
            .then(|| self.config.name.clone().map(ControlValue::Text))
            .flatten()
    }

    fn set_param(
        &mut self,
        name: &str,
        value: &ControlValue,
    ) -> std::result::Result<ParamEffect, ControlError> {
        if name != "monitor" || !self.display {
            return Err(ControlError::Unknown(name.to_string()));
        }
        let monitor = crate::source::expect_text(name, value)?;
        self.config.name = Some(monitor.to_string());
        // Placing needs the monitor list; the caller patches with the new
        // config, which does.
        Ok(ParamEffect::Rebuild)
    }

    fn window_request(&self) -> Option<winit::window::WindowAttributes> {
        if self.bound.is_some() || (self.display && self.config.name.is_none()) {
            return None;
        }
        let mut attrs = Window::default_attributes();
        if let Some([w, h]) = self.config.size {
            attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h));
        }
        if let Some([x, y]) = self.config.position {
            attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(x, y));
        }
        Some(attrs)
    }

    fn attach_window(
        &mut self,
        gpu: &GpuContext,
        services: &crate::source::Services,
        window: Window,
    ) -> Result<()> {
        let window = Arc::new(window);
        let size = window.inner_size();
        let surface = gpu
            .instance
            .create_surface(Arc::clone(&window))
            .context("Failed to create output surface")?;
        let capabilities = surface.get_capabilities(&gpu.adapter);
        let selection = select_surface_presentation(PresentationRequest::default(), &capabilities)?;
        // Immediate for the lowest latency to projectors, so an output window
        // never throttles the main loop through vsync contention.
        let present_mode = if capabilities
            .present_modes
            .contains(&wgpu::PresentMode::Immediate)
        {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::Fifo
        };
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: selection.format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: capabilities.alpha_modes[0],
            color_space: selection.color_space,
            view_formats: vec![],
            desired_maximum_frame_latency: 3,
        };
        surface.configure(&gpu.device, &surface_config);
        // macOS ignores the position hint and configuring can move the window,
        // so the saved position is applied last.
        if let Some([x, y]) = self.config.position {
            window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
        }
        self.bound = Some(Bound {
            window,
            surface,
            surface_config,
            capabilities,
        });
        self.place(services.get::<crate::output::window::Monitors>());
        Ok(())
    }

    fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    fn window_id(&self) -> Option<winit::window::WindowId> {
        self.bound.as_ref().map(|b| b.window.id())
    }

    fn window_resized(&mut self, gpu: &GpuContext, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if let Some(bound) = &mut self.bound {
            bound.surface_config.width = width;
            bound.surface_config.height = height;
            bound.surface.configure(&gpu.device, &bound.surface_config);
        }
    }

    fn set_title(&mut self, title: &str) {
        if let Some(bound) = &self.bound {
            bound.window.set_title(title);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl Drop for WindowSink {
    /// Drop the surface before the window it draws into.
    fn drop(&mut self) {
        if let Some(bound) = self.bound.take() {
            drop(bound.surface);
            drop(bound.window);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(config: &SinkConfig) -> Box<dyn OutputSinkInstance> {
        let config: Config = crate::source::decode_config(config).unwrap();
        Box::new(WindowSink::new(config, true))
    }

    #[test]
    fn a_new_display_opens_no_window_until_a_monitor_is_chosen() {
        let mut sink = display(&SinkConfig::new(DISPLAY));
        assert_eq!(sink.sink_type(), DISPLAY);
        assert!(sink.window_request().is_none());
        assert_eq!(sink.param("monitor"), None);

        assert!(matches!(
            sink.set_param("monitor", &ControlValue::Text("Studio".into())),
            Ok(ParamEffect::Rebuild)
        ));
        assert!(sink.patch(&sink.config(), &crate::source::Services::new()));
        assert!(sink.window_request().is_some());
        assert_eq!(sink.config().str("name"), Some("Studio"));
    }

    #[test]
    fn a_saved_display_asks_for_its_window() {
        let sink = display(&SinkConfig::new(DISPLAY).with("name", "Studio"));
        assert!(sink.window_request().is_some());
    }

    #[test]
    fn switching_to_a_window_drops_the_monitor() {
        let mut sink = display(&SinkConfig::new(DISPLAY).with("name", "Studio"));
        assert!(sink.patch(&SinkConfig::new(WINDOWED), &crate::source::Services::new()));
        assert_eq!(sink.sink_type(), WINDOWED);
        assert_eq!(sink.config().str("name"), None);
        assert!(sink.schema().is_empty());
    }
}
