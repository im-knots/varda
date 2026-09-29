//! Discovery, receive, and publish for Spout.
//!
//! The API compiles everywhere and reports unavailable off Windows, so
//! persistence, API and UI code build and test on every platform. Only the
//! bodies are Windows-gated.

#[cfg(target_os = "windows")]
use std::collections::HashMap;

/// A source Varda can receive from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpoutSource {
    pub name: String,
}

/// Discovery, receivers, and publishers.
///
/// Render-thread only, like the Syphon manager.
pub struct SpoutManager {
    available: bool,
    sources: Vec<SpoutSource>,
    receivers: Vec<Receiver>,
    textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    #[cfg(target_os = "windows")]
    bridge: Option<super::d3d::Bridge>,
    #[cfg(target_os = "windows")]
    senders: HashMap<String, Sender>,
    /// `Rgba8UnormSrgb` to BGRA conversion for the publish path, built once.
    #[cfg(target_os = "windows")]
    convert: Option<crate::renderer::blit::BlitPipeline>,
}

/// One receiver: which sender, and what it last published.
struct Receiver {
    name: String,
    /// The shared handle currently bound; zero until the first frame. A change
    /// (sender restarted or resized) rebuilds the destination texture.
    handle: u32,
    width: u32,
    height: u32,
    connected: bool,
}

/// One publisher: the shared texture receivers read, plus the registry entries
/// keeping it discoverable.
///
/// The two `SharedMemory` handles are only held: Windows frees a mapping when its
/// last handle closes, so dropping either unregisters the sender.
#[cfg(target_os = "windows")]
struct Sender {
    shared: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    /// Where the conversion blit lands before the bridge copies it across.
    /// One texture, no ring: the blit and the bridge copy go to the same D3D12
    /// queue, so they are ordered without a fence (assumed, untested).
    staging: wgpu::Texture,
    /// An sRGB view of `staging`, so the hardware applies the transfer function,
    /// as on the Syphon publish path.
    staging_view: wgpu::TextureView,
    width: u32,
    height: u32,
    _registration: super::sharedmem::SharedMemory,
    _info: super::sharedmem::SharedMemory,
}

impl Default for SpoutManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SpoutManager {
    /// Spout is available on Windows, and only when wgpu is on the Dx12 backend.
    ///
    /// The backend cannot be known until a device exists, so this reports what
    /// the platform allows and [`Self::ensure_bridge`] settles the rest.
    #[must_use]
    pub fn new() -> Self {
        Self::with_available(cfg!(target_os = "windows"))
    }

    /// A manager that will never do anything, for `--no-spout`.
    #[must_use]
    pub fn new_disabled() -> Self {
        Self::with_available(false)
    }

    fn with_available(available: bool) -> Self {
        Self {
            available,
            sources: Vec::new(),
            receivers: Vec::new(),
            textures: Vec::new(),
            #[cfg(target_os = "windows")]
            bridge: None,
            #[cfg(target_os = "windows")]
            senders: HashMap::new(),
            #[cfg(target_os = "windows")]
            convert: None,
        }
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.available
    }

    #[must_use]
    pub fn sources(&self) -> &[SpoutSource] {
        &self.sources
    }

    #[must_use]
    pub fn discovered_sources(&self) -> Vec<String> {
        self.sources.iter().map(|s| s.name.clone()).collect()
    }

    /// Re-read the machine's sender registry.
    #[cfg(target_os = "windows")]
    pub fn discover(&mut self) {
        if !self.available {
            return;
        }
        self.sources = super::sharedmem::sender_names()
            .into_iter()
            .map(|name| SpoutSource { name })
            .collect();
    }

    /// No senders exist off Windows.
    #[cfg(not(target_os = "windows"))]
    pub fn discover(&mut self) {}

    /// Begin receiving from a sender, returning its index.
    ///
    /// Succeeds before the sender publishes anything: the deck shows a
    /// placeholder until frames arrive, so start order does not matter.
    pub fn start_receive(&mut self, name: &str, device: &wgpu::Device) -> Option<usize> {
        if !self.available {
            return None;
        }
        let (width, height) = (1920, 1080);
        let (texture, view) = make_placeholder(device, name, width, height);
        let idx = self.receivers.len();
        self.receivers.push(Receiver {
            name: name.to_string(),
            handle: 0,
            width,
            height,
            connected: false,
        });
        self.textures.push((texture, view));
        Some(idx)
    }

    pub fn stop_receive(&mut self, idx: usize) {
        if let Some(r) = self.receivers.get_mut(idx) {
            r.handle = 0;
            r.connected = false;
        }
    }

    #[must_use]
    pub fn texture_view(&self, idx: usize) -> Option<&wgpu::TextureView> {
        self.textures.get(idx).map(|(_, v)| v)
    }

    /// Which sender a receiver is bound to.
    ///
    /// Used to late-bind decks whose producer was not running when the scene
    /// loaded, as the Syphon path does by name.
    #[must_use]
    pub fn source_name(&self, idx: usize) -> Option<&str> {
        self.receivers.get(idx).map(|r| r.name.as_str())
    }

    #[must_use]
    pub fn client_dimensions(&self, idx: usize) -> Option<(u32, u32)> {
        self.receivers.get(idx).map(|r| (r.width, r.height))
    }

    #[must_use]
    pub fn is_connected(&self, idx: usize) -> bool {
        self.receivers.get(idx).is_some_and(|r| r.connected)
    }

    /// Pull the current frame from every sender being received.
    ///
    /// Render thread. The destination is rebuilt only when the sender's handle or
    /// size changes; otherwise each frame is one GPU copy across the bridge.
    #[cfg(target_os = "windows")]
    pub fn update(&mut self, device: &wgpu::Device) {
        if !self.available || self.receivers.is_empty() {
            return;
        }
        // Taken out so the loop can hold `&mut self.textures` at the same time.
        self.ensure_bridge(device);
        let Some(bridge) = self.bridge.take() else {
            return;
        };
        for i in 0..self.receivers.len() {
            let name = self.receivers[i].name.clone();
            let Some(info) = super::sharedmem::sender_info(&name) else {
                self.receivers[i].connected = false;
                continue;
            };
            let format = super::protocol::DxgiFormat::from_u32_or_default(info.format);
            let Some(format) = format else {
                // Sampling an unknown layout gives a plausible wrong picture.
                log::warn!(
                    "Spout '{name}': sender publishes unsupported format {}",
                    info.format
                );
                self.receivers[i].connected = false;
                continue;
            };
            let changed = info.share_handle != self.receivers[i].handle
                || info.width != self.receivers[i].width
                || info.height != self.receivers[i].height;
            if changed {
                self.textures[i] =
                    make_receive_texture(device, &name, info.width, info.height, format);
                self.receivers[i].handle = info.share_handle;
                self.receivers[i].width = info.width;
                self.receivers[i].height = info.height;
                log::debug!(
                    "Spout '{name}': bound {}x{} {format:?}",
                    info.width,
                    info.height
                );
            }
            self.receivers[i].connected =
                bridge.copy_from_sender(info.share_handle, &self.textures[i].0);
        }
        self.bridge = Some(bridge);
    }

    /// Nothing to pull off Windows.
    #[cfg(not(target_os = "windows"))]
    pub fn update(&mut self, _device: &wgpu::Device) {}

    /// Publish a composited frame as a Spout sender.
    ///
    /// Blits into a `BGRA8` staging texture through an sRGB view, then copies it
    /// across the bridge into the shared texture. The blit is submitted first so
    /// both are ordered on one queue.
    #[cfg(target_os = "windows")]
    pub fn publish_frame_gpu(
        &mut self,
        context: &crate::renderer::context::GpuContext,
        name: &str,
        source: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        if !self.available || width == 0 || height == 0 {
            return;
        }
        self.ensure_bridge(&context.device);
        if self.bridge.is_none() {
            return;
        }
        if !self.ensure_sender(&context.device, name, width, height) {
            return;
        }
        let Some(sender) = self.senders.get(name) else {
            return;
        };
        let Some(pipeline) = self.convert.as_ref() else {
            return;
        };

        let bind = pipeline.create_bind_group(&context.device, source);
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Spout Convert Encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Spout Convert Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &sender.staging_view,
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
            pipeline.render(&mut pass, &bind);
        }
        context.submit(std::iter::once(encoder.finish()));

        if let Some(bridge) = self.bridge.as_ref() {
            bridge.copy_to_shared(&sender.staging, &sender.shared);
        }
    }

    /// Nothing to publish off Windows.
    #[cfg(not(target_os = "windows"))]
    pub fn publish_frame_gpu(
        &mut self,
        _context: &crate::renderer::context::GpuContext,
        _name: &str,
        _source: &wgpu::TextureView,
        _width: u32,
        _height: u32,
    ) {
    }

    /// Build the bridge once, and remember when it cannot be built.
    ///
    /// If wgpu chose Vulkan the bridge can never be built, so `available` is
    /// cleared instead of retrying every frame.
    #[cfg(target_os = "windows")]
    fn ensure_bridge(&mut self, device: &wgpu::Device) {
        if self.bridge.is_some() || !self.available {
            return;
        }
        self.bridge = super::d3d::Bridge::new(device);
        if self.bridge.is_none() {
            log::warn!("Spout needs the Dx12 backend; disabling on this adapter");
            self.available = false;
        }
    }

    /// Create or resize the shared texture and its registry entries.
    #[cfg(target_os = "windows")]
    fn ensure_sender(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        width: u32,
        height: u32,
    ) -> bool {
        if self
            .senders
            .get(name)
            .is_some_and(|s| s.width == width && s.height == height)
        {
            return true;
        }
        let format = super::protocol::DxgiFormat::Bgra8Unorm;
        let Some(bridge) = self.bridge.as_ref() else {
            return false;
        };
        let Some((shared, handle)) = bridge.create_shared(width, height, format) else {
            log::error!("Spout '{name}': could not create the shared texture");
            return false;
        };
        let Some(registration) = super::sharedmem::register_sender(name) else {
            log::error!("Spout '{name}': the sender table is full");
            return false;
        };
        let info = super::protocol::SharedTextureInfo {
            share_handle: handle,
            width,
            height,
            format: format.as_u32(),
            usage: 0,
            partner_id: 0,
        };
        let Some(published) = super::sharedmem::publish_sender_info(name, &info) else {
            log::error!("Spout '{name}': could not publish the texture description");
            return false;
        };
        let (staging, staging_view) = make_publish_texture(device, name, width, height);
        if self.convert.is_none() {
            self.convert = crate::renderer::blit::BlitPipeline::new(
                device,
                wgpu::TextureFormat::Bgra8UnormSrgb,
            )
            .ok();
        }
        self.senders.insert(
            name.to_string(),
            Sender {
                shared,
                staging,
                staging_view,
                width,
                height,
                _registration: registration,
                _info: published,
            },
        );
        log::info!("Spout publishing as '{name}' at {width}x{height}");
        true
    }
}

/// A black stand-in bound until a sender publishes.
///
/// Decks bind when added, possibly before the producer starts, so
/// `texture_view` always has something to return.
fn make_placeholder(
    device: &wgpu::Device,
    name: &str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&format!("Spout Receiver Placeholder: {name}")),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// A destination sized and formatted for what a sender publishes.
#[cfg(target_os = "windows")]
fn make_receive_texture(
    device: &wgpu::Device,
    name: &str,
    width: u32,
    height: u32,
    format: super::protocol::DxgiFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&format!("Spout Receiver: {name}")),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: format.wgpu_format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// The staging texture the conversion blit renders into.
///
/// `Bgra8Unorm` with a `Bgra8UnormSrgb` view. The shared texture is
/// `DXGI_FORMAT_B8G8R8A8_UNORM` and the bridge copies bytes as-is, so the blit
/// applies the transfer function, in hardware via the view.
#[cfg(target_os = "windows")]
fn make_publish_texture(
    device: &wgpu::Device,
    name: &str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&format!("Spout Sender: {name}")),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[wgpu::TextureFormat::Bgra8UnormSrgb],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Spout Sender sRGB View"),
        format: Some(wgpu::TextureFormat::Bgra8UnormSrgb),
        ..Default::default()
    });
    (texture, view)
}

impl crate::source::ShareReceiver for SpoutManager {
    fn is_available(&self) -> bool {
        SpoutManager::is_available(self)
    }
    fn server_names(&self) -> Vec<String> {
        self.discovered_sources()
    }
    fn discover(&mut self) {
        SpoutManager::discover(self);
    }
    fn start_receive(&mut self, name: &str, device: &wgpu::Device) -> Option<usize> {
        SpoutManager::start_receive(self, name, device)
    }
    fn stop_receive(&mut self, client: usize) {
        SpoutManager::stop_receive(self, client);
    }
    fn update(&mut self, device: &wgpu::Device) {
        SpoutManager::update(self, device);
    }
    fn texture_view(&self, client: usize) -> Option<&wgpu::TextureView> {
        SpoutManager::texture_view(self, client)
    }
    fn client_dimensions(&self, client: usize) -> Option<(u32, u32)> {
        SpoutManager::client_dimensions(self, client)
    }
    fn is_connected(&self, client: usize) -> bool {
        SpoutManager::is_connected(self, client)
    }
}

/// Spout senders as a deck source. Not listed off Windows.
pub fn provider() -> crate::source::ShareProvider<SpoutManager> {
    crate::source::ShareProvider::new(crate::source::ShareProtocol {
        id: "Spout",
        label: "Spout Senders",
        unavailable: "Spout is available on Windows only",
    })
}

impl crate::output::share::ShareSender for SpoutManager {
    fn is_available(&self) -> bool {
        SpoutManager::is_available(self)
    }

    fn publish(
        &mut self,
        gpu: &crate::renderer::context::GpuContext,
        name: &str,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        self.publish_frame_gpu(gpu, name, view, width, height);
    }
}

/// Spout outputs: a sender other Windows applications read the output from.
static SPOUT_OUTPUT: crate::output::share::ShareOutput = crate::output::share::ShareOutput {
    id: "spout_sender",
    label: "Spout",
    icon: "🔗",
    name_field: "sender_name",
    unavailable: "Spout is available on Windows only",
    presentation: crate::delivery::presentation::spout_presentation,
};

/// The Spout output type.
pub fn sink_provider() -> crate::output::share::ShareSinkProvider<SpoutManager> {
    crate::output::share::ShareSinkProvider::new(&SPOUT_OUTPUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Integration, persistence and UI code bind against this API, so it must
    // behave on platforms without Spout.

    #[test]
    fn spout_is_unavailable_off_windows() {
        let mgr = SpoutManager::new();
        assert_eq!(mgr.is_available(), cfg!(target_os = "windows"));
    }

    #[test]
    fn a_disabled_manager_is_unavailable_everywhere() {
        let mgr = SpoutManager::new_disabled();
        assert!(!mgr.is_available());
        assert!(mgr.sources().is_empty());
    }

    #[test]
    fn discovery_on_an_unavailable_manager_finds_nothing_and_does_not_panic() {
        let mut mgr = SpoutManager::new_disabled();
        mgr.discover();
        assert!(mgr.discovered_sources().is_empty());
    }

    #[test]
    fn queries_for_receivers_that_do_not_exist_are_none() {
        let mgr = SpoutManager::new();
        assert!(mgr.texture_view(0).is_none());
        assert!(mgr.client_dimensions(0).is_none());
        assert!(!mgr.is_connected(0));
        assert!(mgr.texture_view(999).is_none());
    }

    #[test]
    fn a_receiver_remembers_which_sender_it_wants() {
        // Late binding needs the name kept until the producer appears.
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let mut mgr = SpoutManager::new();
        if !mgr.is_available() {
            assert!(
                mgr.start_receive("Anything", &context.device).is_none(),
                "an unavailable manager must not hand out receiver indices"
            );
            return;
        }
        let idx = mgr
            .start_receive("Producer", &context.device)
            .expect("receiver");
        assert_eq!(mgr.source_name(idx), Some("Producer"));
        assert!(
            mgr.texture_view(idx).is_some(),
            "a placeholder is bound immediately"
        );
        assert!(
            !mgr.is_connected(idx),
            "not connected until the sender publishes"
        );
    }

    #[test]
    fn stopping_a_receiver_that_does_not_exist_is_a_no_op() {
        let mut mgr = SpoutManager::new_disabled();
        mgr.stop_receive(0);
        mgr.stop_receive(999);
    }
}
