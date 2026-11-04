//! Shows rendered frames in a window from a thread of its own, so the frame
//! loop never waits for a display to take a frame.
//!
//! The render thread draws a window's frame into a texture it owns, submits,
//! then hands the frame to [`Presenter::publish`]. The presenter thread
//! acquires the window's next drawable, blits the newest frame into it and
//! presents. Submission order is the handoff: the queue runs submits in order,
//! so a blit submitted after the frame's commands reads the finished frame.
//! Frames published while the thread waits for the display replace each
//! other, so it always shows the newest.
//!
//! This relies on wgpu not holding a device-wide lock while it waits for a
//! drawable: wgpu-core 30 locks only the surface being acquired.

use super::blit::BlitPipeline;
use super::context::GpuContext;
use super::submit_gate::SubmitGate;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

/// What a presenter thread drives: one window's surface, or a fake in tests.
pub trait PresentTarget: Send + 'static {
    /// A finished frame to show.
    type Frame: Send + 'static;
    /// Surface settings, applied before the next frame.
    type Settings: Send + 'static;

    fn apply(&mut self, settings: Self::Settings);

    /// Show `frame`. May block until the display takes it.
    fn present(&mut self, frame: &Self::Frame);
}

struct State<T: PresentTarget> {
    frame: Option<T::Frame>,
    settings: Option<T::Settings>,
    closed: bool,
}

struct Shared<T: PresentTarget> {
    state: Mutex<State<T>>,
    wake: Condvar,
}

impl<T: PresentTarget> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The handle to a presenter thread. Dropping it ends the thread after the
/// frame it is presenting, and the thread drops its target.
pub struct Presenter<T: PresentTarget> {
    shared: Arc<Shared<T>>,
}

impl<T: PresentTarget> Presenter<T> {
    /// Start a thread named `name` presenting to `target`.
    ///
    /// # Errors
    ///
    /// Fails when the thread cannot be spawned.
    pub fn spawn(name: &str, target: T) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                frame: None,
                settings: None,
                closed: false,
            }),
            wake: Condvar::new(),
        });
        let thread_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || run(&thread_shared, target))?;
        Ok(Self { shared })
    }

    /// Hand over a frame whose commands are already submitted. Replaces a
    /// frame the thread has not started on yet. Never waits for the display.
    pub fn publish(&self, frame: T::Frame) {
        self.shared.lock().frame = Some(frame);
        self.shared.wake.notify_one();
    }

    /// Change the surface settings before the next frame.
    pub fn configure(&self, settings: T::Settings) {
        self.shared.lock().settings = Some(settings);
        self.shared.wake.notify_one();
    }
}

impl<T: PresentTarget> Drop for Presenter<T> {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.wake.notify_one();
    }
}

fn run<T: PresentTarget>(shared: &Shared<T>, mut target: T) {
    loop {
        let (settings, frame) = {
            let mut state = shared.lock();
            while !state.closed && state.frame.is_none() && state.settings.is_none() {
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            if state.closed {
                return;
            }
            (state.settings.take(), state.frame.take())
        };
        if let Some(settings) = settings {
            target.apply(settings);
        }
        if let Some(frame) = frame {
            target.present(&frame);
        }
    }
}

/// A window's surface as a presenter target. Frames are views of textures in
/// the surface's format; one of another size is stretched to fill.
pub struct SurfaceTarget {
    device: wgpu::Device,
    queue: wgpu::Queue,
    gate: SubmitGate,
    config: wgpu::SurfaceConfiguration,
    blit: BlitPipeline,
    label: String,
    // Declared before `_window` so it drops first: the surface draws into it.
    surface: wgpu::Surface<'static>,
    /// Keeps the window alive until the surface is gone.
    _window: Option<Arc<winit::window::Window>>,
}

impl SurfaceTarget {
    /// Configure `surface` with `config` and present to it. `window` is held
    /// until the surface is gone, when the caller does not keep it alive.
    ///
    /// # Errors
    ///
    /// Fails when the blit pipeline cannot be built.
    pub fn new(
        gpu: &GpuContext,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        window: Option<Arc<winit::window::Window>>,
        label: &str,
    ) -> anyhow::Result<Self> {
        gpu.submit_gate.configure(&surface, &gpu.device, &config);
        let blit = BlitPipeline::new(&gpu.device, config.format)?;
        Ok(Self {
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
            gate: gpu.submit_gate.clone(),
            config,
            blit,
            label: label.to_owned(),
            surface,
            _window: window,
        })
    }

    fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Some(texture),
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.gate
                    .configure(&self.surface, &self.device, &self.config);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(texture)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Some(texture),
                    other => {
                        log::warn!("{}: no drawable after reconfiguring: {other:?}", self.label);
                        None
                    }
                }
            }
            other => {
                log::debug!("{}: drawable unavailable: {other:?}", self.label);
                None
            }
        }
    }
}

impl PresentTarget for SurfaceTarget {
    type Frame = wgpu::TextureView;
    type Settings = wgpu::SurfaceConfiguration;

    fn apply(&mut self, config: wgpu::SurfaceConfiguration) {
        if config == self.config || config.width == 0 || config.height == 0 {
            return;
        }
        if config.format != self.config.format {
            match BlitPipeline::new(&self.device, config.format) {
                Ok(blit) => self.blit = blit,
                Err(e) => {
                    log::error!(
                        "{}: cannot present in {:?}: {e:#}",
                        self.label,
                        config.format
                    );
                    return;
                }
            }
        }
        self.config = config;
        self.gate
            .configure(&self.surface, &self.device, &self.config);
    }

    fn present(&mut self, frame: &wgpu::TextureView) {
        let Some(drawable) = self.acquire() else {
            return;
        };
        let view = drawable
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Present Encoder"),
            });
        blit_frame(&self.device, &mut encoder, &self.blit, frame, &view);
        self.gate
            .submit(&self.queue, std::iter::once(encoder.finish()));
        self.queue.present(drawable);
    }
}

/// Copy `source` into `target`, stretched to fill it.
fn blit_frame(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    blit: &BlitPipeline,
    source: &wgpu::TextureView,
    target: &wgpu::TextureView,
) {
    let bind_group = blit.create_bind_group(device, source);
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Present Blit"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
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
    blit.render(&mut pass, &bind_group);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(5);

    #[derive(Debug, PartialEq)]
    enum Event {
        Applied(u32),
        Presented(u32),
    }

    /// Reports what it is asked to do. While `gate` is set, each present
    /// waits for a message on it, as a display that has no drawable free.
    struct Fake {
        events: mpsc::Sender<Event>,
        gate: Option<mpsc::Receiver<()>>,
        dropped: mpsc::Sender<()>,
    }

    impl PresentTarget for Fake {
        type Frame = u32;
        type Settings = u32;

        fn apply(&mut self, settings: u32) {
            self.events.send(Event::Applied(settings)).unwrap();
        }

        fn present(&mut self, frame: &u32) {
            self.events.send(Event::Presented(*frame)).unwrap();
            if let Some(gate) = &self.gate {
                gate.recv_timeout(WAIT).unwrap();
            }
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = self.dropped.send(());
        }
    }

    struct Rig {
        presenter: Presenter<Fake>,
        events: mpsc::Receiver<Event>,
        dropped: mpsc::Receiver<()>,
    }

    fn rig(gate: Option<mpsc::Receiver<()>>) -> Rig {
        let (events_tx, events) = mpsc::channel();
        let (dropped_tx, dropped) = mpsc::channel();
        let fake = Fake {
            events: events_tx,
            gate,
            dropped: dropped_tx,
        };
        Rig {
            presenter: Presenter::spawn("present-test", fake).unwrap(),
            events,
            dropped,
        }
    }

    #[test]
    fn frames_published_while_the_display_is_busy_collapse_to_the_newest() {
        let (open, gate) = mpsc::channel();
        let rig = rig(Some(gate));

        rig.presenter.publish(1);
        assert_eq!(rig.events.recv_timeout(WAIT).unwrap(), Event::Presented(1));
        // The display holds frame 1. Publishing does not wait for it.
        for frame in 2..=4 {
            rig.presenter.publish(frame);
        }
        open.send(()).unwrap();
        assert_eq!(rig.events.recv_timeout(WAIT).unwrap(), Event::Presented(4));
        open.send(()).unwrap();
        assert!(rig.events.recv_timeout(Duration::from_millis(50)).is_err());
    }

    #[test]
    fn settings_apply_before_the_next_frame() {
        let rig = rig(None);
        rig.presenter.configure(7);
        rig.presenter.publish(1);
        let first = rig.events.recv_timeout(WAIT).unwrap();
        let second = rig.events.recv_timeout(WAIT).unwrap();
        assert_eq!([first, second], [Event::Applied(7), Event::Presented(1)]);
    }

    #[test]
    fn dropping_the_handle_ends_the_thread_and_drops_the_target() {
        let rig = rig(None);
        drop(rig.presenter);
        rig.dropped.recv_timeout(WAIT).unwrap();
    }

    fn texture(gpu: &crate::renderer::context::GpuContext, size: u32) -> wgpu::Texture {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("present test"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// The blit copies a frame exactly at the same size, and stretches it into
    /// a drawable of another size.
    #[test]
    fn the_blit_copies_a_frame_into_a_drawable_of_any_size() {
        const SIZE: u32 = 4;
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let pixels: Vec<u8> = (0..SIZE * SIZE)
            .flat_map(|i| {
                let v = (i * 37) as u8;
                [v, v.wrapping_mul(3), v.wrapping_mul(7), 255]
            })
            .collect();
        let frame = texture(&gpu, SIZE);
        gpu.queue.write_texture(
            frame.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: Some(SIZE),
            },
            frame.size(),
        );
        let frame_view = frame.create_view(&wgpu::TextureViewDescriptor::default());
        let blit = BlitPipeline::new(&gpu.device, wgpu::TextureFormat::Bgra8UnormSrgb).unwrap();

        let draw = |size: u32| {
            let target = texture(&gpu, size);
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            blit_frame(&gpu.device, &mut encoder, &blit, &frame_view, &view);
            gpu.queue.submit(std::iter::once(encoder.finish()));
            crate::testing::read_rgba8(&gpu, &target)
        };

        assert_eq!(draw(SIZE), pixels);

        // At double size the corner texel samples only the source's corner.
        let doubled = draw(SIZE * 2);
        assert_eq!(doubled.len(), (SIZE * SIZE * 16) as usize);
        assert_eq!(doubled[..4], pixels[..4]);
    }
}
