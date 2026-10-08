//! Analyzers: frame analysis for modulation and shader preprocessing.

pub(crate) mod brightness;
mod capture;
#[cfg(feature = "face-detection")]
pub(crate) mod face_detect;
pub(crate) mod fractal_flight;
pub(crate) mod host_inline;
pub(crate) mod traits;

pub(crate) use host_inline::HostInlineSet;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};

use traits::{
    Analyzer, AnalyzerInput, AnalyzerSchema, AnalyzerSnapshot, AnalyzerStateSnapshot,
    HostInlinePreprocessor,
};

// ── Registry ────────────────────────────────────────────────────────────────

type AnalyzerFactory = Box<dyn Fn() -> Box<dyn Analyzer> + Send + Sync>;
type HostInlineFactory = Box<dyn Fn() -> Box<dyn HostInlinePreprocessor> + Send + Sync>;

/// Execution category of a preprocessor type. Shaders declare every category
/// the same way in `PREPROCESSORS`; the category decides how the engine runs it
/// and whether the shader can load without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreprocessorCategory {
    /// Worker thread reading a CPU readback of the deck's frame and publishing
    /// `AnalyzerSnapshot`s. Optional: falls back to default outputs.
    CpuAnalyzer,
    /// GPU passes reading textures from an external device manager. The device
    /// is acquired at load; if unavailable, the shader does not load.
    GpuDeviceBacked,
    /// Stepped on the render thread once per rendered frame, before the
    /// shader. One instance per deck. Optional.
    HostInline,
    // GPU passes over the deck's own frame (e.g. edge detect) are not
    // implemented; they need a frame-input path no preprocessor uses yet.
}

impl PreprocessorCategory {
    /// Whether this category runs as GPU passes instead of an analyzer thread.
    /// GPU categories have no factory and never go to `DeckAnalyzers`.
    pub(crate) fn is_gpu(self) -> bool {
        matches!(self, Self::GpuDeviceBacked)
    }

    /// Whether a shader declaring this type fails to load when it is unavailable.
    pub(crate) fn is_required(self) -> bool {
        matches!(self, Self::GpuDeviceBacked)
    }
}

/// Available preprocessor types, built at startup.
pub(crate) struct AnalyzerRegistry {
    factories: HashMap<String, AnalyzerFactory>,
    host_inline_factories: HashMap<String, HostInlineFactory>,
    schemas: HashMap<String, AnalyzerSchema>,
    categories: HashMap<String, PreprocessorCategory>,
}

impl AnalyzerRegistry {
    pub(crate) fn new() -> Self {
        Self {
            factories: HashMap::new(),
            host_inline_factories: HashMap::new(),
            schemas: HashMap::new(),
            categories: HashMap::new(),
        }
    }

    /// Registers a CPU analyzer type with a factory.
    pub(crate) fn register<F>(mut self, analyzer_type: &str, factory: F) -> Self
    where
        F: Fn() -> Box<dyn Analyzer> + Send + Sync + 'static,
    {
        let instance = factory();
        let schema = instance.output_schema();
        self.schemas.insert(analyzer_type.to_owned(), schema);
        self.factories
            .insert(analyzer_type.to_owned(), Box::new(factory));
        self.categories
            .insert(analyzer_type.to_owned(), PreprocessorCategory::CpuAnalyzer);
        self
    }

    /// Registers a GPU preprocessor type. It has no factory or worker thread
    /// (the deck render path runs its passes), so the schema is passed in.
    pub(crate) fn register_gpu(
        mut self,
        preprocessor_type: &str,
        category: PreprocessorCategory,
        schema: AnalyzerSchema,
    ) -> Self {
        debug_assert!(
            category.is_gpu(),
            "register_gpu called with a CPU category for '{preprocessor_type}'"
        );
        self.schemas.insert(preprocessor_type.to_owned(), schema);
        self.categories
            .insert(preprocessor_type.to_owned(), category);
        self
    }

    /// Registers a host-inline preprocessor type with a factory.
    pub(crate) fn register_host_inline<F>(mut self, preprocessor_type: &str, factory: F) -> Self
    where
        F: Fn() -> Box<dyn HostInlinePreprocessor> + Send + Sync + 'static,
    {
        let schema = factory().output_schema();
        self.schemas.insert(preprocessor_type.to_owned(), schema);
        self.host_inline_factories
            .insert(preprocessor_type.to_owned(), Box::new(factory));
        self.categories.insert(
            preprocessor_type.to_owned(),
            PreprocessorCategory::HostInline,
        );
        self
    }

    /// New instance of a host-inline type. `None` for any other category.
    pub(crate) fn create_host_inline(
        &self,
        preprocessor_type: &str,
    ) -> Option<Box<dyn HostInlinePreprocessor>> {
        self.host_inline_factories
            .get(preprocessor_type)
            .map(|f| f())
    }

    /// New instance of `analyzer_type`. `None` for GPU categories, which have
    /// no factory.
    pub(crate) fn create(&self, analyzer_type: &str) -> Option<Box<dyn Analyzer>> {
        self.factories.get(analyzer_type).map(|f| f())
    }

    /// Every registered preprocessor type name, GPU and CPU.
    pub(crate) fn available_types(&self) -> Vec<&str> {
        self.categories.keys().map(String::as_str).collect()
    }

    /// Output schema for a registered preprocessor type.
    pub(crate) fn schema_for(&self, analyzer_type: &str) -> Option<&AnalyzerSchema> {
        self.schemas.get(analyzer_type)
    }

    /// Execution category for a registered preprocessor type.
    pub(crate) fn category_for(&self, preprocessor_type: &str) -> Option<PreprocessorCategory> {
        self.categories.get(preprocessor_type).copied()
    }
}

// ── Per-Deck Instance Management ────────────────────────────────────────────

struct AnalyzerInstance {
    refcount: usize,
    /// Long side of the frame this analyzer reads, or `None` if it reads no
    /// pixels. Stored here because the analyzer moves onto its worker thread.
    frame_size: Option<u32>,
    thread: Option<JoinHandle<()>>,
    latest: Arc<ArcSwap<AnalyzerSnapshot>>,
    stop: Arc<AtomicBool>,
    frame_tx: Sender<AnalyzerInput>,
    /// Disconnects when the worker thread exits (the thread holds the sender).
    /// Lets shutdown stop it with a bounded, non-blocking wait.
    done_rx: Receiver<()>,
}

/// How long to wait for an analyzer worker to exit before detaching it, so a
/// thread stuck in blocking FFI (e.g. ONNX Runtime) can't freeze shutdown.
const STOP_GRACE: Duration = Duration::from_secs(2);

/// Running analyzer instances for one deck.
pub(crate) struct DeckAnalyzers {
    instances: HashMap<String, AnalyzerInstance>,
    /// Created on the first `capture_frame` call that needs pixels.
    capture: Option<capture::FrameCapture>,
}

impl DeckAnalyzers {
    pub(crate) fn new() -> Self {
        Self {
            instances: HashMap::new(),
            capture: None,
        }
    }

    /// Requests an analyzer type, or increments its refcount if running.
    pub(crate) fn request(
        &mut self,
        analyzer_type: &str,
        registry: &AnalyzerRegistry,
        options: &serde_json::Value,
    ) -> Option<Arc<ArcSwap<AnalyzerSnapshot>>> {
        if let Some(inst) = self.instances.get_mut(analyzer_type) {
            inst.refcount += 1;
            log::debug!("Analyzer '{analyzer_type}' refcount -> {}", inst.refcount);
            return Some(Arc::clone(&inst.latest));
        }

        let analyzer = registry.create(analyzer_type)?;

        // The schema needs no init(), so the default snapshot can be built
        // before the worker runs.
        let schema = analyzer.output_schema();
        let frame_size = analyzer.frame_size();
        let initial = AnalyzerSnapshot::from_defaults(&schema);
        let latest = Arc::new(ArcSwap::from_pointee(initial));
        let stop = Arc::new(AtomicBool::new(false));
        let (frame_tx, frame_rx) = crossbeam_channel::bounded(2);
        let (done_tx, done_rx) = crossbeam_channel::bounded::<()>(0);

        let thread_latest = Arc::clone(&latest);
        let thread_stop = Arc::clone(&stop);
        let type_name = analyzer_type.to_owned();
        // init() can be slow (e.g. loading ONNX models), so it runs on the
        // worker thread.
        let options = options.clone();

        let thread = std::thread::Builder::new()
            .name(format!("analyzer-{type_name}"))
            .spawn(move || {
                // Dropped when the thread exits, even by panic, which
                // disconnects `done_rx`.
                let _done = done_tx;
                analyzer_thread(
                    analyzer,
                    &options,
                    &frame_rx,
                    &thread_latest,
                    &thread_stop,
                    &type_name,
                );
            })
            .ok()?;

        log::info!("Spawned analyzer '{analyzer_type}'");
        let handle = Arc::clone(&latest);
        self.instances.insert(
            analyzer_type.to_owned(),
            AnalyzerInstance {
                refcount: 1,
                frame_size,
                thread: Some(thread),
                latest,
                stop,
                frame_tx,
                done_rx,
            },
        );
        Some(handle)
    }

    /// Releases a reference. Stops the analyzer at refcount zero.
    pub(crate) fn release(&mut self, analyzer_type: &str) {
        let should_remove = if let Some(inst) = self.instances.get_mut(analyzer_type) {
            inst.refcount = inst.refcount.saturating_sub(1);
            inst.refcount == 0
        } else {
            false
        };

        if should_remove && let Some(inst) = self.instances.remove(analyzer_type) {
            stop_instance(inst, analyzer_type, "");
        }
    }

    /// Latest snapshot for an analyzer type.
    pub(crate) fn latest_snapshot(
        &self,
        analyzer_type: &str,
    ) -> Option<arc_swap::Guard<Arc<AnalyzerSnapshot>>> {
        self.instances
            .get(analyzer_type)
            .map(|inst| inst.latest.load())
    }

    /// All active snapshots as (`analyzer_type`, snapshot).
    pub(crate) fn all_snapshots(
        &self,
    ) -> impl Iterator<Item = (String, arc_swap::Guard<Arc<AnalyzerSnapshot>>)> + '_ {
        self.instances
            .iter()
            .map(|(k, inst)| (k.clone(), inst.latest.load()))
    }

    /// Removes instances whose worker has exited (e.g. `init()` failed because
    /// ONNX Runtime is missing). Otherwise the render loop keeps doing a GPU
    /// readback per frame and logging "channel disconnected". Costs one
    /// non-blocking `try_recv` per instance.
    fn prune_dead(&mut self) {
        let dead: Vec<String> = self
            .instances
            .iter()
            .filter(|(_, inst)| matches!(inst.done_rx.try_recv(), Err(TryRecvError::Disconnected)))
            .map(|(name, _)| name.clone())
            .collect();

        for name in dead {
            if let Some(mut inst) = self.instances.remove(&name) {
                if let Some(thread) = inst.thread.take() {
                    let _ = thread.join();
                }
                log::warn!("Analyzer '{name}' worker exited; removing instance");
            }
        }
    }

    /// Reduces the deck texture for analysis and delivers an earlier frame's
    /// pixels to the analyzers. Call from the render loop after effects.
    /// Returns the capture's command buffer, or `None` if no analyzer can take
    /// a frame.
    pub(crate) fn capture_frame(
        &mut self,
        device: &wgpu::Device,
        source_texture: &wgpu::Texture,
        states: &HashMap<String, AnalyzerStateSnapshot>,
    ) -> Option<wgpu::CommandBuffer> {
        self.prune_dead();
        if self.instances.is_empty() {
            return None;
        }
        let source_size = (source_texture.width(), source_texture.height());

        // Analyzers that read no pixels still get the deck's size, since
        // geometry-only analyzers check camera packets against the render aspect.
        let Some(long_side) = self.instances.values().filter_map(|i| i.frame_size).max() else {
            self.deliver(&Self::placeholder(source_size), states);
            return None;
        };
        let size = capture::capture_size(source_size, long_side);
        if !self
            .capture
            .as_ref()
            .is_some_and(|c| c.fits(source_size, size))
        {
            self.capture = Some(capture::FrameCapture::new(device, source_size, size));
        }
        let capture = self.capture.as_mut()?;

        if let Some(frame) = capture.try_read(device) {
            let input = AnalyzerInput {
                frame: Arc::new(frame.into_bytes()),
                width: size.0,
                height: size.1,
                timestamp: std::time::Instant::now(),
                state: AnalyzerStateSnapshot::default(),
            };
            self.deliver(&input, states);
        }

        let room = self
            .instances
            .values()
            .any(|i| i.frame_size.is_some() && !i.frame_tx.is_full());
        if !room {
            return None;
        }
        let capture = self.capture.as_mut()?;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Analyzer capture"),
        });
        capture.encode(device, &mut encoder, source_texture);
        Some(encoder.finish())
    }

    /// An input with the deck's size and no pixels.
    fn placeholder((width, height): (u32, u32)) -> AnalyzerInput {
        AnalyzerInput {
            frame: Arc::new(Vec::new()),
            width,
            height,
            timestamp: std::time::Instant::now(),
            state: AnalyzerStateSnapshot::default(),
        }
    }

    /// Sends `input` to every analyzer that reads pixels, and a placeholder of
    /// its size to the others. Each gets its own bound state. Non-blocking; a
    /// full channel drops the frame.
    fn deliver(&self, input: &AnalyzerInput, states: &HashMap<String, AnalyzerStateSnapshot>) {
        let frameless = AnalyzerInput {
            frame: Arc::new(Vec::new()),
            ..input.clone()
        };
        for (name, inst) in &self.instances {
            let mut payload = if inst.frame_size.is_some() {
                input.clone()
            } else {
                frameless.clone()
            };
            payload.state = states.get(name).cloned().unwrap_or_default();
            match inst.frame_tx.try_send(payload) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    log::warn!("Analyzer '{name}' channel disconnected");
                }
            }
        }
    }

    /// Whether any analyzer instance is running.
    pub(crate) fn has_active_instances(&self) -> bool {
        !self.instances.is_empty()
    }

    pub(crate) fn running_types(&self) -> Vec<String> {
        self.instances.keys().cloned().collect()
    }

    /// Stops every running instance.
    pub(crate) fn shutdown(&mut self) {
        let types: Vec<String> = self.instances.keys().cloned().collect();
        for t in types {
            if let Some(inst) = self.instances.remove(&t) {
                stop_instance(inst, &t, " (deck shutdown)");
            }
        }
    }
}

impl Drop for DeckAnalyzers {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ── Analyzer Thread ─────────────────────────────────────────────────────────

fn analyzer_thread(
    mut analyzer: Box<dyn Analyzer>,
    options: &serde_json::Value,
    frame_rx: &Receiver<AnalyzerInput>,
    latest: &ArcSwap<AnalyzerSnapshot>,
    stop: &AtomicBool,
    type_name: &str,
) {
    // On init failure the thread exits and the deck keeps the default snapshot.
    if let Err(e) = analyzer.init(options) {
        log::error!("Failed to init analyzer '{type_name}': {e}");
        return;
    }
    log::info!("Analyzer thread '{type_name}' started");
    while !stop.load(Ordering::Relaxed) {
        match frame_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(input) => match analyzer.analyze(&input) {
                Ok(snapshot) => {
                    latest.store(Arc::new(snapshot));
                }
                Err(e) => {
                    log::error!("Analyzer '{type_name}' error: {e}");
                }
            },
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
    analyzer.shutdown();
    log::info!("Analyzer thread '{type_name}' stopped");
}

/// Stops one analyzer with a bounded wait. Waits up to [`STOP_GRACE`] for the
/// worker to exit and joins it; otherwise (e.g. stuck in FFI) detaches it so
/// shutdown never freezes.
fn stop_instance(mut inst: AnalyzerInstance, type_name: &str, suffix: &str) {
    inst.stop.store(true, Ordering::Relaxed);
    drop(inst.frame_tx);

    let exited = !matches!(
        inst.done_rx.recv_timeout(STOP_GRACE),
        Err(crossbeam_channel::RecvTimeoutError::Timeout)
    );

    if exited {
        if let Some(thread) = inst.thread.take() {
            let _ = thread.join();
        }
        log::info!("Stopped analyzer '{type_name}'{suffix}");
    } else {
        // Detach the stuck thread; the OS reclaims it at exit.
        let _ = inst.thread.take();
        log::warn!("Analyzer '{type_name}'{suffix} did not stop within {STOP_GRACE:?}; detaching");
    }
}

// ── Default Registry ────────────────────────────────────────────────────────

/// Default registry with every built-in analyzer.
pub(crate) fn default_registry() -> AnalyzerRegistry {
    #[allow(unused_mut)]
    let mut registry = AnalyzerRegistry::new()
        .register("brightness", || {
            Box::new(brightness::BrightnessAnalyzer::new())
        })
        .register_host_inline(fractal_flight::PREPROCESSOR_TYPE, || {
            Box::new(fractal_flight::FractalFlight::new())
        });
    #[cfg(feature = "face-detection")]
    {
        registry = registry.register("face_detect", || {
            Box::new(face_detect::FaceDetectAnalyzer::new())
        });
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// A frame-consuming analyzer on a color-path deck must encode a legal
    /// capture. Submitting the encoder is the assertion, since that is where
    /// validation runs.
    #[test]
    fn color_path_deck_capture_encodes_a_legal_pass() {
        let Some(context) = crate::testing::headless_gpu() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("color path deck"),
            size: wgpu::Extent3d {
                width: 64,
                height: 36,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::renderer::context::COLOR_PATH_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let registry = default_registry();
        let mut deck = DeckAnalyzers::new();
        deck.request("brightness", &registry, &serde_json::Value::Null)
            .expect("brightness analyzer starts");

        let command = deck
            .capture_frame(&context.device, &texture, &HashMap::new())
            .expect("a frame-consuming analyzer encodes a capture");
        context.queue.submit(std::iter::once(command));
        let _ = context.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }

    /// Records every input it gets. With a gate, each `analyze` waits for a
    /// message on it, so its channel fills.
    struct Recorder {
        size: Option<u32>,
        inputs: Sender<AnalyzerInput>,
        gate: Option<Receiver<()>>,
    }

    impl Analyzer for Recorder {
        fn analyzer_type(&self) -> &'static str {
            "recorder"
        }

        fn output_schema(&self) -> AnalyzerSchema {
            AnalyzerSchema {
                scalars: Vec::new(),
                textures: Vec::new(),
            }
        }

        fn init(&mut self, _options: &serde_json::Value) -> anyhow::Result<()> {
            Ok(())
        }

        fn frame_size(&self) -> Option<u32> {
            self.size
        }

        fn analyze(&mut self, input: &AnalyzerInput) -> anyhow::Result<AnalyzerSnapshot> {
            if let Some(gate) = &self.gate {
                let _ = gate.recv();
            }
            let _ = self.inputs.send(input.clone());
            Ok(AnalyzerSnapshot::from_defaults(&self.output_schema()))
        }
    }

    /// A registry of recorders, one per `(name, size)`, all sending to `inputs`.
    fn recorders(
        sizes: &[(&'static str, Option<u32>)],
        inputs: &Sender<AnalyzerInput>,
        gate: Option<&Receiver<()>>,
    ) -> AnalyzerRegistry {
        sizes
            .iter()
            .fold(AnalyzerRegistry::new(), |registry, &(name, size)| {
                let inputs = inputs.clone();
                let gate = gate.cloned();
                registry.register(name, move || {
                    Box::new(Recorder {
                        size,
                        inputs: inputs.clone(),
                        gate: gate.clone(),
                    }) as Box<dyn Analyzer>
                })
            })
    }

    /// A color-path deck texture of linear RGBA pixels, row-major.
    fn deck_texture(
        context: &crate::renderer::context::GpuContext,
        (width, height): (u32, u32),
        pixel: impl Fn(u32, u32) -> [f32; 4],
    ) -> wgpu::Texture {
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("analyzer test deck"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::renderer::context::COLOR_PATH_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bytes: Vec<u8> = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .flat_map(|(x, y)| pixel(x, y))
            .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
            .collect();
        context.queue.write_texture(
            texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 8),
                rows_per_image: Some(height),
            },
            texture.size(),
        );
        texture
    }

    /// Capture and drain the GPU until `count` inputs arrive, or panic.
    fn capture_inputs(
        context: &crate::renderer::context::GpuContext,
        deck: &mut DeckAnalyzers,
        texture: &wgpu::Texture,
        inputs: &Receiver<AnalyzerInput>,
        count: usize,
    ) -> Vec<AnalyzerInput> {
        let mut received = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while received.len() < count {
            assert!(
                Instant::now() < deadline,
                "only {} inputs arrived",
                received.len()
            );
            if let Some(command) = deck.capture_frame(&context.device, texture, &HashMap::new()) {
                context.queue.submit(std::iter::once(command));
            }
            let _ = context.device.poll(wgpu::PollType::wait_indefinitely());
            received.extend(inputs.try_iter().filter(|i| !i.frame.is_empty()));
            std::thread::sleep(Duration::from_millis(5));
        }
        received
    }

    /// Linear light arrives display-encoded, scaled to the analyzer's long side.
    #[test]
    fn a_capture_arrives_display_encoded_at_the_analyzers_size() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let registry = recorders(&[("recorder", Some(16))], &tx, None);
        let mut deck = DeckAnalyzers::new();
        deck.request("recorder", &registry, &serde_json::Value::Null)
            .expect("recorder starts");
        let texture = deck_texture(&context, (64, 36), |_, _| [0.5, 0.5, 0.5, 1.0]);

        let input = capture_inputs(&context, &mut deck, &texture, &rx, 1).remove(0);
        assert_eq!((input.width, input.height), (16, 9));
        assert_eq!(input.frame.len(), 16 * 9 * 4);
        // Linear 0.5 is sRGB 0.735; alpha stays linear.
        for pixel in input.frame.as_chunks::<4>().0 {
            for &channel in &pixel[..3] {
                assert!((187..=189).contains(&channel), "{pixel:?}");
            }
            assert_eq!(pixel[3], 255);
        }
    }

    /// Each output pixel averages its whole footprint, so detail between
    /// sample points still counts.
    #[test]
    fn a_reduced_capture_averages_its_footprint() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let registry = recorders(&[("recorder", Some(8))], &tx, None);
        let mut deck = DeckAnalyzers::new();
        deck.request("recorder", &registry, &serde_json::Value::Null)
            .expect("recorder starts");
        // White in the first two columns of every eight: a quarter of the light.
        let texture = deck_texture(&context, (64, 64), |x, _| {
            let v = if x % 8 < 2 { 1.0 } else { 0.0 };
            [v, v, v, 1.0]
        });

        let input = capture_inputs(&context, &mut deck, &texture, &rx, 1).remove(0);
        assert_eq!((input.width, input.height), (8, 8));
        // Linear 0.25 is sRGB 0.537.
        for pixel in input.frame.as_chunks::<4>().0 {
            assert!((133..=141).contains(&pixel[0]), "{pixel:?}");
        }
    }

    /// A deck smaller than every analyzer's long side is read at its own size.
    #[test]
    fn a_capture_never_upscales() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let registry = recorders(&[("recorder", Some(1920))], &tx, None);
        let mut deck = DeckAnalyzers::new();
        deck.request("recorder", &registry, &serde_json::Value::Null)
            .expect("recorder starts");
        let texture = deck_texture(&context, (64, 36), |_, _| [1.0, 0.0, 0.0, 1.0]);

        let input = capture_inputs(&context, &mut deck, &texture, &rx, 1).remove(0);
        assert_eq!((input.width, input.height), (64, 36));
        assert_eq!(&input.frame[..4], &[255, 0, 0, 255]);
    }

    /// Analyzers on one deck share one frame at the largest size any asks for.
    #[test]
    fn analyzers_on_a_deck_share_one_frame() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let registry = recorders(&[("small", Some(8)), ("large", Some(32))], &tx, None);
        let mut deck = DeckAnalyzers::new();
        for name in ["small", "large"] {
            deck.request(name, &registry, &serde_json::Value::Null)
                .expect("recorder starts");
        }
        let texture = deck_texture(&context, (64, 32), |_, _| [0.0, 1.0, 0.0, 1.0]);

        let inputs = capture_inputs(&context, &mut deck, &texture, &rx, 2);
        assert_eq!((inputs[0].width, inputs[0].height), (32, 16));
        assert!(Arc::ptr_eq(&inputs[0].frame, &inputs[1].frame));
    }

    /// When no analyzer can take a frame, the deck encodes no capture.
    #[test]
    fn a_full_channel_skips_the_capture() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let (tx, _rx) = crossbeam_channel::unbounded();
        let (open, gate) = crossbeam_channel::unbounded();
        let registry = recorders(&[("recorder", Some(16))], &tx, Some(&gate));
        let mut deck = DeckAnalyzers::new();
        deck.request("recorder", &registry, &serde_json::Value::Null)
            .expect("recorder starts");
        let texture = deck_texture(&context, (64, 36), |_, _| [0.2, 0.2, 0.2, 1.0]);

        // The worker holds one frame in `analyze` and the channel holds two more.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() < deadline, "the channel never filled");
            let command = deck.capture_frame(&context.device, &texture, &HashMap::new());
            let Some(command) = command else { break };
            context.queue.submit(std::iter::once(command));
            let _ = context.device.poll(wgpu::PollType::wait_indefinitely());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            deck.capture_frame(&context.device, &texture, &HashMap::new())
                .is_none(),
            "still full"
        );
        drop(open);
        deck.shutdown();
    }

    #[test]
    fn registry_builder_pattern() {
        let registry = default_registry();
        let types = registry.available_types();
        assert!(types.contains(&"brightness"));
        assert!(registry.schema_for("brightness").is_some());
        assert!(registry.schema_for("nonexistent").is_none());
    }

    #[test]
    fn registry_create_instance() {
        let registry = default_registry();
        let instance = registry.create("brightness");
        assert!(instance.is_some());
        assert_eq!(instance.unwrap().analyzer_type(), "brightness");
    }

    #[test]
    fn deck_analyzers_lifecycle() {
        let registry = default_registry();
        let mut deck = DeckAnalyzers::new();

        let handle = deck
            .request("brightness", &registry, &serde_json::Value::Null)
            .expect("should create");
        assert!(deck.has_active_instances());

        let handle2 = deck
            .request("brightness", &registry, &serde_json::Value::Null)
            .expect("should reuse");
        let _ = (handle, handle2);

        deck.release("brightness");
        assert!(deck.has_active_instances());

        deck.release("brightness");
        assert!(!deck.has_active_instances());
    }

    #[test]
    fn deck_analyzers_send_and_read() {
        let registry = default_registry();
        let mut deck = DeckAnalyzers::new();

        let _handle = deck
            .request("brightness", &registry, &serde_json::Value::Null)
            .expect("should create");

        let input = AnalyzerInput {
            frame: Arc::new(vec![255u8; 4 * 4 * 4]),
            width: 4,
            height: 4,
            timestamp: Instant::now(),
            state: AnalyzerStateSnapshot::default(),
        };
        deck.deliver(&input, &HashMap::new());
        std::thread::sleep(Duration::from_millis(200));

        let snapshot = deck
            .latest_snapshot("brightness")
            .expect("should have snapshot");
        let brightness = snapshot.scalar("brightness");
        assert!(
            brightness > 0.9,
            "expected brightness ~1.0, got {brightness}"
        );
        deck.shutdown();
    }

    #[cfg(feature = "face-detection")]
    #[test]
    fn dead_worker_is_pruned() {
        // A missing model file makes face_detect's init() fail and the worker
        // exit. The instance must be pruned, and shutdown must stay fast.
        let registry = default_registry();
        let mut deck = DeckAnalyzers::new();
        let opts = serde_json::json!({
            "model_path": "/nonexistent/__varda_missing_model__.onnx"
        });
        let _handle = deck
            .request("face_detect", &registry, &opts)
            .expect("should spawn worker");

        // Poll prune_dead() instead of one sleep: under load the failing init()
        // may not have finished. The timeout still fails a real hang.
        let deadline = Instant::now() + Duration::from_secs(10);
        while deck.has_active_instances() && Instant::now() < deadline {
            deck.prune_dead();
            if !deck.has_active_instances() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !deck.has_active_instances(),
            "dead worker instance should be pruned"
        );

        let start = Instant::now();
        deck.shutdown();
        assert!(
            start.elapsed() < STOP_GRACE,
            "shutdown must be fast when the worker already exited, took {:?}",
            start.elapsed()
        );
    }
}
