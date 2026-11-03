//! The UI window's frame. The runner draws each frame into a texture it owns
//! and a presenter thread shows it, so the frame loop never waits for the
//! display.

use crate::renderer::context::{GpuContext, WindowSurface};
use crate::renderer::presenter::{Presenter, SurfaceTarget};

pub(super) struct UiWindow {
    presenter: Presenter<SurfaceTarget>,
    config: wgpu::SurfaceConfiguration,
    view: wgpu::TextureView,
    /// Set while the OS reports the window fully hidden; nothing is drawn.
    pub(super) occluded: bool,
}

impl UiWindow {
    /// Move `surface` to a presenter thread and allocate the first frame.
    ///
    /// # Errors
    ///
    /// Fails when the presenter's pipeline or thread cannot be created.
    pub(super) fn new(gpu: &GpuContext, surface: WindowSurface) -> anyhow::Result<Self> {
        let WindowSurface {
            surface,
            surface_config: config,
        } = surface;
        let target = SurfaceTarget::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            surface,
            config.clone(),
            None,
            "UI window",
        )?;
        let presenter = Presenter::spawn("present-ui", target)?;
        let view = frame_view(&gpu.device, &config);
        Ok(Self {
            presenter,
            config,
            view,
            occluded: false,
        })
    }

    /// Follow a window resize. The presenter stretches a frame of the old size
    /// until the next one is drawn.
    pub(super) fn resize(&mut self, device: &wgpu::Device, size: winit::dpi::PhysicalSize<u32>) {
        if size.width == 0
            || size.height == 0
            || (size.width, size.height) == (self.config.width, self.config.height)
        {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.view = frame_view(device, &self.config);
        self.presenter.configure(self.config.clone());
    }

    /// The texture this frame draws into.
    pub(super) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub(super) fn size(&self) -> [u32; 2] {
        [self.config.width, self.config.height]
    }

    /// Show the frame. Call after its commands are submitted.
    pub(super) fn publish(&self) {
        self.presenter.publish(self.view.clone());
    }
}

fn frame_view(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("UI Frame"),
            size: wgpu::Extent3d {
                width: config.width.max(1),
                height: config.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}
