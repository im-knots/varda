//! Test helpers. Integration tests get them through the `test-fixtures`
//! feature, which `[dev-dependencies]` enables.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh workspace root for one `VardaApp`.
///
/// Every test that builds a `VardaApp` must pass this to `--workspace`.
/// Otherwise [`crate::app::AppConfig`] uses the current directory's `.varda/`,
/// which under `cargo test` is the crate root and may hold a developer's real
/// show data.
///
/// Each call returns a new directory under one per-process temp root. The
/// root is never cleaned up, since no [`tempfile::TempDir`] guard has an owner.
///
/// # Panics
///
/// Panics if the directory cannot be created or its path is not UTF-8.
pub fn temp_workspace() -> String {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let root = ROOT.get_or_init(|| {
        tempfile::Builder::new()
            .prefix("varda-test-workspace-")
            .tempdir()
            .expect("create the per-process test workspace root")
    });

    let dir: PathBuf = root
        .path()
        .join(format!("ws{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).expect("create a test workspace");
    dir.into_os_string()
        .into_string()
        .expect("temporary directory path is valid UTF-8")
}

/// A headless GPU context, or `None` when this machine has no adapter.
///
/// Under `VARDA_REQUIRE_GPU=1` (set in CI) a missing adapter panics instead.
///
/// # Panics
///
/// Panics if no context can be created while `VARDA_REQUIRE_GPU` is set.
pub fn headless_gpu() -> Option<crate::renderer::context::GpuContext> {
    match crate::renderer::context::GpuContext::new_headless() {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            assert!(
                std::env::var_os("VARDA_REQUIRE_GPU").is_none(),
                "VARDA_REQUIRE_GPU is set but no headless GPU context is available: {e:#}"
            );
            eprintln!("no GPU adapter, skipping");
            None
        }
    }
}

/// A headless GPU context on a hardware adapter, or `None` on a CPU rasterizer.
///
/// For tests too heavy for WARP or lavapipe, such as the raymarched fractal
/// explorer: CI's runners have only those, and a frame there costs minutes.
/// Shader translation and pipeline creation stay covered by the guards that
/// build every shader.
///
/// # Panics
///
/// Panics if no context can be created while `VARDA_REQUIRE_GPU` is set.
pub fn hardware_gpu() -> Option<crate::renderer::context::GpuContext> {
    let gpu = headless_gpu()?;
    let info = gpu.adapter.get_info();
    if info.device_type == wgpu::DeviceType::Cpu {
        eprintln!("{} is a CPU rasterizer, skipping", info.name);
        return None;
    }
    Some(gpu)
}

/// Render one frame of `source` into a new `width` by `height` target of the
/// compositing format and read it back as linear RGBA, row-major.
///
/// # Panics
///
/// Panics if rendering or the readback fails.
pub fn render_source_pixels(
    gpu: &crate::renderer::context::GpuContext,
    source: &mut dyn crate::source::DeckSourceInstance,
    width: u32,
    height: u32,
) -> Vec<[f32; 4]> {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test source target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    assert_eq!(gpu.compositing_format, wgpu::TextureFormat::Rgba16Float);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let audio = crate::audio::AudioData::default();
    let modulation = crate::modulation::ModulationEngine::new();
    let mut params = crate::params::ShaderParams::from_inputs(&[]);
    let mut cmd_buffers = Vec::new();
    let mut frame = crate::source::SourceFrame {
        gpu,
        target: &view,
        target_texture: &texture,
        width,
        height,
        transparent: false,
        time: 0.0,
        time_delta: 0.0,
        frame_index: 0,
        phase_times: [0.0; 4],
        live: false,
        audio: &audio,
        modulation: &modulation,
        param_prefix: "deck/test/param",
        params: &mut params,
        cmd_buffers: &mut cmd_buffers,
    };
    source.render(&mut frame).expect("source renders");
    gpu.queue.submit(cmd_buffers);

    read_rgba16f(gpu, &texture, width, height)
}

/// Read an `Rgba16Float` texture back as linear RGBA, row-major. Blocks on
/// the GPU.
///
/// # Panics
///
/// Panics if the readback cannot be mapped.
pub fn read_rgba16f(
    gpu: &crate::renderer::context::GpuContext,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<[f32; 4]> {
    let bytes_per_pixel = 8u32;
    let padded = (width * bytes_per_pixel).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: u64::from(padded * height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(std::iter::once(encoder.finish()));
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .ok();
    rx.recv().expect("map channel").expect("map ok");
    let mut out = Vec::with_capacity((width * height) as usize);
    {
        let data = buffer
            .slice(..)
            .get_mapped_range()
            .expect("mapped readback");
        for row in 0..height {
            let base = (row * padded) as usize;
            for col in 0..width {
                let px = base + (col * bytes_per_pixel) as usize;
                out.push(std::array::from_fn(|i| {
                    half::f16::from_le_bytes([data[px + i * 2], data[px + i * 2 + 1]]).to_f32()
                }));
            }
        }
    }
    buffer.unmap();
    out
}

/// The sRGB 8-bit value of a linear channel, unrounded, as a PNG would store it.
pub fn srgb8(value: f32) -> f64 {
    let v = f64::from(value.clamp(0.0, 1.0));
    let encoded = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    encoded * 255.0
}

/// Set a numeric shader input through the setter that matches its declared
/// type: `long` truncates, `bool` is true above 0.5, and an `event` fires above 0.5.
///
/// # Errors
///
/// Fails if the shader has no input `name`, or it is not a float, long, bool or event.
pub fn set_param(
    params: &mut crate::params::ShaderParams,
    name: &str,
    value: f64,
) -> anyhow::Result<()> {
    let Some(definition) = params.definitions.get(name) else {
        anyhow::bail!("shader has no input `{name}`");
    };
    match definition.input_type.as_str() {
        "float" => params.set_float(name, value as f32),
        "long" => params.set_long(name, value as i32),
        "bool" | "event" => params.set_bool(name, value > 0.5),
        other => anyhow::bail!("`{name}` is a {other}, not a number"),
    }
    Ok(())
}

/// One acceptance scene from `tests/fixtures/fractal_scenes.json`.
pub struct FractalScene {
    pub name: String,
    /// Shader inputs, in file order.
    pub params: Vec<(String, f64)>,
    /// Preprocessor state, such as the camera pose.
    pub state: serde_json::Map<String, serde_json::Value>,
}

/// The fractal explorer's acceptance scenes.
///
/// # Panics
///
/// Panics if the fixture is missing or malformed.
pub fn fractal_scenes() -> Vec<FractalScene> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/fractal_scenes.json"
    );
    let text = std::fs::read_to_string(path).expect("scene fixture");
    let list: Vec<serde_json::Value> = serde_json::from_str(&text).expect("scene JSON");
    list.into_iter()
        .map(|scene| FractalScene {
            name: scene["name"].as_str().expect("name").to_owned(),
            params: scene["params"]
                .as_object()
                .expect("params")
                .iter()
                .map(|(k, v)| (k.clone(), v.as_f64().expect("numeric param")))
                .collect(),
            state: scene["state"].as_object().expect("state").clone(),
        })
        .collect()
}

/// An engine built with `config` on a headless GPU, or `None` when there is no GPU.
///
/// # Panics
///
/// Panics if the engine cannot be built on an available GPU.
pub fn headless_app_with(config: &crate::app::AppConfig) -> Option<crate::app::VardaApp> {
    let gpu = headless_gpu()?;
    Some(crate::app::VardaApp::new(gpu, config).expect("the engine builds on a headless GPU"))
}

/// An engine on a headless GPU with a scratch workspace.
///
/// # Panics
///
/// Panics if the engine cannot be built on an available GPU.
pub fn headless_app() -> Option<crate::app::VardaApp> {
    headless_app_with(&headless_config())
}

/// Config for a test-owned headless engine: no OSC, NDI, or Syphon, and a
/// workspace from [`temp_workspace`]. Use this rather than listing flags by hand.
///
/// # Panics
///
/// Panics if the scratch workspace cannot be created.
pub fn headless_config() -> crate::app::AppConfig {
    use clap::Parser;
    crate::app::AppConfig::parse_from([
        "varda",
        "--headless",
        "--no-osc",
        "--no-ndi",
        "--no-syphon",
        "--workspace",
        &temp_workspace(),
    ])
}

/// One 8-bit NDI send per frame, split so a benchmark can time the two render
/// thread steps without GPU waits: `encode` submits the GPU work, `read`
/// collects the finished UYVY bytes.
pub struct NdiSendBench {
    context: crate::renderer::context::GpuContext,
    view: wgpu::TextureView,
    ndi: crate::ndi::NdiManager,
    width: u32,
    height: u32,
}

impl NdiSendBench {
    const SENDER: &'static str = "bench";

    /// A `width` x `height` rendered output holding a gradient.
    pub fn new(context: crate::renderer::context::GpuContext, width: u32, height: u32) -> Self {
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("NDI send bench output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                let x = i % width;
                let y = i / width;
                [
                    (x * 255 / width) as u8,
                    (y * 255 / height) as u8,
                    ((x + y) % 256) as u8,
                    255,
                ]
            })
            .collect();
        context.queue.write_texture(
            texture.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            context,
            view,
            ndi: crate::ndi::NdiManager::new_disabled(),
            width,
            height,
        }
    }

    /// Record and submit this frame's GPU work.
    ///
    /// # Panics
    ///
    /// Panics if the frame cannot be converted.
    pub fn encode(&mut self) {
        let mut encoder = self
            .context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.ndi
            .begin_frame(
                Self::SENDER,
                crate::ndi::FrameConversion {
                    device: &self.context.device,
                    queue: &self.context.queue,
                    encoder: &mut encoder,
                    source: &self.view,
                    width: self.width,
                    height: self.height,
                    dither: true,
                    pixel_format: crate::engine::value::render::PresentationPixelFormat::Uyvy,
                },
            )
            .expect("an even-width UYVY frame converts");
        self.context.submit(std::iter::once(encoder.finish()));
    }

    /// Collect a finished frame as UYVY bytes, if one is ready.
    pub fn read(&mut self) -> Option<usize> {
        self.ndi.read_converted(Self::SENDER, &self.context.device)
    }

    /// Let the GPU finish what has been submitted. Not render-thread time.
    pub fn wait(&self) {
        let _ = self
            .context
            .device
            .poll(wgpu::PollType::wait_indefinitely());
    }
}

/// One NDI receiver fed a synthetic 1080p frame in the SDK's pixel format, for
/// benchmarking the receive path.
pub struct NdiReceiveBench {
    context: crate::renderer::context::GpuContext,
    ndi: crate::ndi::NdiManager,
    receiver: usize,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

impl NdiReceiveBench {
    /// A receiver and a `width` x `height` UYVY frame (a source without alpha).
    pub fn new(context: crate::renderer::context::GpuContext, width: u32, height: u32) -> Self {
        let mut ndi = crate::ndi::NdiManager::new_disabled();
        let receiver = ndi.add_test_receiver(&context.device);
        let pixels = (0..width / 2 * height)
            .flat_map(|i| {
                let x = i % (width / 2);
                let y = i / (width / 2);
                [
                    (16 + x * 224 / width) as u8,
                    (16 + y * 219 / height) as u8,
                    (240 - x * 224 / width) as u8,
                    (16 + (x + y) % 220) as u8,
                ]
            })
            .collect();
        Self {
            context,
            ndi,
            receiver,
            pixels,
            width,
            height,
        }
    }

    /// The receive thread's work for one captured frame.
    pub fn receive(&mut self) {
        let frame = crate::ndi::ffi::NDIlib_video_frame_v2_t {
            xres: self.width.cast_signed(),
            yres: self.height.cast_signed(),
            FourCC: crate::ndi::ffi::NDIlib_FourCC_video_type_e::UYVY,
            frame_rate_N: 60,
            frame_rate_D: 1,
            picture_aspect_ratio: 0.0,
            frame_format_type: 1,
            timecode: 0,
            p_data: self.pixels.as_mut_ptr(),
            line_stride_in_bytes: (self.width * 2).cast_signed(),
            p_metadata: std::ptr::null(),
            timestamp: 0,
        };
        self.ndi.receive_for_test(self.receiver, &frame);
    }

    /// The render thread's work to put the received frame on the GPU.
    pub fn upload(&mut self) {
        if let Some(conversion) = self.ndi.update(&self.context.device, &self.context.queue) {
            self.context.submit(std::iter::once(conversion));
        }
    }

    /// Let the GPU finish what has been submitted. Not render-thread time.
    pub fn wait(&self) {
        let _ = self
            .context
            .device
            .poll(wgpu::PollType::wait_indefinitely());
    }
}

/// The render thread's side of handing a read-back 1080p frame to a
/// recording's writer queue: one queue being drained, and one that is full.
pub struct RecordingFeedBench {
    accepting: std::sync::mpsc::SyncSender<Vec<u8>>,
    full: std::sync::mpsc::SyncSender<Vec<u8>>,
    _full_rx: std::sync::mpsc::Receiver<Vec<u8>>,
    dropped: std::sync::atomic::AtomicU64,
    width: u32,
    height: u32,
}

impl RecordingFeedBench {
    /// # Panics
    ///
    /// Panics if the writer thread cannot be spawned.
    pub fn new(width: u32, height: u32) -> Self {
        let (accepting, drained) = std::sync::mpsc::sync_channel::<Vec<u8>>(2);
        std::thread::Builder::new()
            .name("recording-feed-bench".into())
            .spawn(move || while drained.recv().is_ok() {})
            .expect("spawn writer");
        let (full, full_rx) = std::sync::mpsc::sync_channel(1);
        let _ = full.send(Vec::new());
        Self {
            accepting,
            full,
            _full_rx: full_rx,
            dropped: std::sync::atomic::AtomicU64::new(0),
            width,
            height,
        }
    }

    /// A read-back frame.
    pub fn frame(&self) -> crate::renderer::ReadbackFrame {
        crate::renderer::ReadbackFrame::rgba8(
            self.width,
            self.height,
            vec![128; (self.width * self.height * 4) as usize],
        )
    }

    /// Hand `frame` to a queue the writer is keeping up with.
    pub fn feed_accepting(&self, frame: crate::renderer::ReadbackFrame) -> bool {
        crate::delivery::ffmpeg::try_enqueue_frame(
            &self.accepting,
            &self.dropped,
            "bench",
            frame.into_bytes(),
        )
    }

    /// Hand `frame` to a queue that is full, so the frame is dropped.
    pub fn feed_full(&self, frame: crate::renderer::ReadbackFrame) -> bool {
        crate::delivery::ffmpeg::try_enqueue_frame(
            &self.full,
            &self.dropped,
            "bench",
            frame.into_bytes(),
        )
    }
}

/// An input source's capture callback with no device, fed one 256-frame
/// stereo buffer at a time, with the render thread's side drained.
pub struct AudioCaptureBench {
    capture: crate::audio::analysis::CaptureState,
    buffer: Vec<f32>,
}

impl AudioCaptureBench {
    /// # Panics
    ///
    /// Panics if the draining or analysis thread cannot be spawned.
    pub fn new() -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(16);
        std::thread::Builder::new()
            .name("audio-capture-bench-drain".into())
            .spawn(move || while receiver.recv().is_ok() {})
            .expect("spawn drain");
        let buffer = (0..crate::audio::AUDIO_BUFFER_SIZE * 2)
            .map(|i| (i as f32 * 0.05).sin() * 0.5)
            .collect();
        Self {
            capture: crate::audio::analysis::CaptureState::new(
                2,
                48_000.0,
                sender,
                std::sync::Arc::default(),
            )
            .expect("spawn analysis"),
            buffer,
        }
    }

    /// The callback's work for one buffer from the device.
    pub fn callback(&mut self) {
        self.capture.process(&self.buffer);
    }
}

impl Default for AudioCaptureBench {
    fn default() -> Self {
        Self::new()
    }
}

/// One hop of a source's analysis.
pub struct HopAnalysisBench {
    analyzer: crate::audio::analysis::HopAnalyzer,
    hop: Vec<f32>,
}

impl HopAnalysisBench {
    pub fn new() -> Self {
        Self {
            analyzer: crate::audio::analysis::HopAnalyzer::new(48_000.0),
            hop: (0..crate::audio::AUDIO_BUFFER_SIZE)
                .map(|i| (i as f32 * 0.05).sin() * 0.5)
                .collect(),
        }
    }

    /// Analyze one hop, returning its level to keep the work observable.
    pub fn analyze(&mut self) -> f32 {
        self.analyzer.analyze(&self.hop).level
    }
}

impl Default for HopAnalysisBench {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the GUI's view of `app` as the windowed runner does each frame, with
/// default layout and no preview textures. Returns the output count so the
/// work is observable.
pub fn gui_view(app: &crate::app::VardaApp) -> usize {
    let engine = app.build_engine_state();
    let layout = crate::usecases::ui::UILayoutState::default();
    let deck = std::collections::HashMap::new();
    let channel = std::collections::HashMap::new();
    let output = std::collections::HashMap::new();
    let textures = crate::usecases::ui::PreviewTextures {
        deck: &deck,
        channel: &channel,
        output: &output,
        main_output: None,
    };
    crate::usecases::ui::build_ui_data(&engine, &layout, &textures, std::sync::Arc::from([]))
        .outputs
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two apps in one process get separate workspaces.
    #[test]
    fn each_call_gets_its_own_directory() {
        let a = temp_workspace();
        let b = temp_workspace();
        assert_ne!(a, b);
        assert!(std::path::Path::new(&a).is_dir());
        assert!(std::path::Path::new(&b).is_dir());
    }

    /// Never the crate root, whose `.varda/` may be real.
    #[test]
    fn never_resolves_to_the_crate_root() {
        let ws = temp_workspace();
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(
            !std::path::Path::new(&ws).starts_with(crate_root),
            "test workspaces must live outside the source tree, got {ws}"
        );
    }

    /// The explicit `--workspace` wins over the current directory's `.varda/`.
    #[test]
    fn headless_config_resolves_to_a_scratch_workspace() {
        let root = headless_config().effective_workspace_root();
        assert!(
            !root.starts_with(env!("CARGO_MANIFEST_DIR")),
            "a test engine must never resolve the source tree as its workspace, got {}",
            root.display()
        );
        assert!(root.is_dir());
    }
}
