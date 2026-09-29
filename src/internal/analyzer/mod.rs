//! Analyzers: frame analysis for modulation and shader preprocessing.

pub(crate) mod brightness;
#[cfg(feature = "face-detection")]
pub(crate) mod face_detect;
pub(crate) mod traits;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};

use traits::{Analyzer, AnalyzerInput, AnalyzerSchema, AnalyzerSnapshot, AnalyzerStateSnapshot};

// ── Registry ────────────────────────────────────────────────────────────────

type AnalyzerFactory = Box<dyn Fn() -> Box<dyn Analyzer> + Send + Sync>;

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
    // GPU passes over the deck's own frame (e.g. edge detect) are not
    // implemented; they need a frame-input path no preprocessor uses yet.
}

impl PreprocessorCategory {
    /// Whether this category runs as GPU passes instead of an analyzer thread.
    /// GPU categories have no factory and never go to `DeckAnalyzers`.
    pub(crate) fn is_gpu(self) -> bool {
        !matches!(self, Self::CpuAnalyzer)
    }

    /// Whether a shader declaring this type fails to load when it is unavailable.
    pub(crate) fn is_required(self) -> bool {
        matches!(self, Self::GpuDeviceBacked)
    }
}

/// Available preprocessor types, built at startup.
pub(crate) struct AnalyzerRegistry {
    factories: HashMap<String, AnalyzerFactory>,
    schemas: HashMap<String, AnalyzerSchema>,
    categories: HashMap<String, PreprocessorCategory>,
}

impl AnalyzerRegistry {
    pub(crate) fn new() -> Self {
        Self {
            factories: HashMap::new(),
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
    /// Whether this analyzer reads the deck's pixels. Stored here because the
    /// analyzer moves onto its worker thread.
    needs_frames: bool,
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
    /// Created on the first `capture_frame` call.
    readback: Option<crate::renderer::ReadbackBuffer>,
    readback_size: (u32, u32),
}

impl DeckAnalyzers {
    pub(crate) fn new() -> Self {
        Self {
            instances: HashMap::new(),
            readback: None,
            readback_size: (0, 0),
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
        let needs_frames = analyzer.needs_frame_input();
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
                needs_frames,
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

    /// Sends a frame to every running analyzer. Non-blocking; drops if full.
    pub(crate) fn send_frame(
        &self,
        input: &AnalyzerInput,
        states: &HashMap<String, AnalyzerStateSnapshot>,
    ) {
        for (name, inst) in &self.instances {
            let mut payload = input.clone();
            payload.state = states.get(name).cloned().unwrap_or_default();
            match inst.frame_tx.try_send(payload) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    log::warn!("Analyzer '{name}' channel disconnected");
                }
            }
        }
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

    /// Copies the deck texture for analysis and delivers the previous frame's
    /// data to the analyzers. Call from the render loop after effects. Returns
    /// the readback command buffer, or `None` if no analyzer needs frames.
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

        // Skip the readback when no analyzer reads pixels: it stalls the
        // pipeline, and its RGBA8 assumption fails validation on a float deck.
        // Analyzers still get source dimensions, since geometry-only analyzers
        // check camera packets against the render aspect.
        if !self.instances.values().any(|i| i.needs_frames) {
            let placeholder = AnalyzerInput {
                frame: Vec::new(),
                width: source_texture.width(),
                height: source_texture.height(),
                timestamp: std::time::Instant::now(),
                state: AnalyzerStateSnapshot::default(),
            };
            self.send_frame(&placeholder, states);
            return None;
        }

        let tex_width = source_texture.width();
        let tex_height = source_texture.height();

        // Read back in the texture's own format; a mismatched row size makes
        // wgpu reject the encoder and the deck gets quarantined.
        let Some(readback_format) = readback_format_for(source_texture.format()) else {
            log::warn!(
                "no analyzer readback for texture format {:?}; frame-consuming \
                 analyzers on this deck will not receive pixels",
                source_texture.format()
            );
            return None;
        };

        // Recreate the readback buffer if size or format changed.
        if self.readback.is_none()
            || self.readback_size != (tex_width, tex_height)
            || self
                .readback
                .as_ref()
                .map(crate::renderer::ReadbackBuffer::format)
                != Some(readback_format)
        {
            self.readback = Some(crate::renderer::ReadbackBuffer::new(
                device,
                tex_width,
                tex_height,
                readback_format,
            ));
            self.readback_size = (tex_width, tex_height);
        }

        // Read the previous frame's data before touching the readback state.
        let prev_frame = self.readback.as_mut().and_then(|rb| rb.try_read(device));

        if let Some(rgba_data) = prev_frame {
            let input = AnalyzerInput {
                // Analyzers always get RGBA8, whatever the deck format.
                frame: frame_to_rgba8(&rgba_data),
                width: self.readback_size.0,
                height: self.readback_size.1,
                timestamp: std::time::Instant::now(),
                state: AnalyzerStateSnapshot::default(),
            };
            for (name, inst) in &self.instances {
                // A frameless analyzer on the same deck is still ticked, but
                // gets no pixels.
                let payload = if inst.needs_frames {
                    input.clone()
                } else {
                    AnalyzerInput {
                        frame: Vec::new(),
                        width: input.width,
                        height: input.height,
                        timestamp: input.timestamp,
                        state: states.get(name).cloned().unwrap_or_default(),
                    }
                };
                let mut payload = payload;
                payload.state = states.get(name).cloned().unwrap_or_default();
                match inst.frame_tx.try_send(payload) {
                    Ok(()) | Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => {
                        log::warn!("Analyzer '{name}' channel disconnected");
                    }
                }
            }
        }

        // Queue this frame's copy, read next frame.
        let readback = self.readback.as_mut().unwrap();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Analyzer readback"),
        });
        readback.begin_readback(&mut encoder, source_texture);
        Some(encoder.finish())
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

/// Readback format matching a deck texture, if analyzers can read it.
fn readback_format_for(format: wgpu::TextureFormat) -> Option<crate::renderer::ReadbackFormat> {
    use crate::renderer::ReadbackFormat as R;
    use wgpu::TextureFormat as F;
    Some(match format {
        F::Rgba8Unorm | F::Rgba8UnormSrgb => R::Rgba8,
        F::Bgra8Unorm | F::Bgra8UnormSrgb => R::Bgra8,
        F::Rgb10a2Unorm => R::Rgb10A2,
        F::Rgba16Float => R::Rgba16Float,
        F::Rgba16Unorm => R::Rgba16Unorm,
        _ => return None,
    })
}

/// One linear-light channel as an eight-bit sRGB sample. Analyzers expect
/// display-encoded values; linear input would darken brightness and face
/// results.
fn linear_to_srgb8(value: f32) -> u8 {
    let v = value.clamp(0.0, 1.0);
    let encoded = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        (encoded * 255.0).round() as u8
    }
}

/// Converts a readback frame to the RGBA8 analyzers expect.
fn frame_to_rgba8(frame: &crate::renderer::ReadbackFrame) -> Vec<u8> {
    use crate::renderer::ReadbackFormat as R;
    let bytes = frame.bytes();
    match frame.format() {
        R::Rgba8 => bytes.to_vec(),
        R::Bgra8 => {
            let mut out = bytes.to_vec();
            for pixel in out.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            out
        }
        R::Rgba16Float => {
            let mut out = Vec::with_capacity(bytes.len() / 2);
            for pixel in bytes.as_chunks::<8>().0 {
                for channel in 0..4 {
                    let raw = u16::from_le_bytes([pixel[channel * 2], pixel[channel * 2 + 1]]);
                    let value = f32::from(half::f16::from_bits(raw));
                    // Alpha is linear; only color channels are encoded.
                    out.push(if channel == 3 {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        {
                            (value.clamp(0.0, 1.0) * 255.0).round() as u8
                        }
                    } else {
                        linear_to_srgb8(value)
                    });
                }
            }
            out
        }
        R::Rgba16Unorm => {
            let mut out = Vec::with_capacity(bytes.len() / 2);
            for pixel in bytes.as_chunks::<8>().0 {
                for channel in 0..4 {
                    let raw = u16::from_le_bytes([pixel[channel * 2], pixel[channel * 2 + 1]]);
                    out.push((raw >> 8) as u8);
                }
            }
            out
        }
        R::Rgb10A2 => {
            let mut out = Vec::with_capacity(bytes.len());
            for pixel in bytes.as_chunks::<4>().0 {
                let word = u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
                out.push(((word & 0x3ff) >> 2) as u8);
                out.push((((word >> 10) & 0x3ff) >> 2) as u8);
                out.push((((word >> 20) & 0x3ff) >> 2) as u8);
                let alpha = ((word >> 30) & 0x3) as u8;
                out.push(alpha * 85);
            }
            out
        }
        // The format gate refuses video layouts before a buffer is built, and
        // `Rgba32Float` is the content light meter's readback, not a deck's.
        R::Uyvy | R::P216 | R::Rgba32Float => Vec::new(),
    }
}

/// Default registry with every built-in analyzer.
pub(crate) fn default_registry() -> AnalyzerRegistry {
    #[allow(unused_mut)]
    let mut registry = AnalyzerRegistry::new().register("brightness", || {
        Box::new(brightness::BrightnessAnalyzer::new())
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
    /// copy. Submitting the encoder is the assertion, since that is where
    /// validation runs.
    #[test]
    fn colour_path_deck_readback_encodes_a_legal_copy() {
        let Some(context) = crate::testing::headless_gpu() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("colour path deck"),
            size: wgpu::Extent3d {
                width: 64,
                height: 36,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::renderer::context::COLOR_PATH_FORMAT,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let registry = default_registry();
        let mut deck = DeckAnalyzers::new();
        deck.request("brightness", &registry, &serde_json::Value::Null)
            .expect("brightness analyzer starts");

        let command = deck
            .capture_frame(&context.device, &texture, &HashMap::new())
            .expect("a frame-consuming analyzer encodes a readback");
        context.queue.submit(std::iter::once(command));
        let _ = context.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }

    /// Whatever the deck's format, analyzers receive eight-bit RGBA.
    #[test]
    fn half_float_frames_convert_to_eight_bit_rgba() {
        assert_eq!(
            readback_format_for(crate::renderer::context::COLOR_PATH_FORMAT),
            Some(crate::renderer::ReadbackFormat::Rgba16Float)
        );
        // Output is display-encoded: linear mid-gray encodes well above mid-gray.
        assert_eq!(linear_to_srgb8(0.0), 0);
        assert_eq!(linear_to_srgb8(1.0), 255);
        assert!(linear_to_srgb8(0.5) > 180, "linear 0.5 encodes bright");
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
            frame: vec![255u8; 4 * 4 * 4],
            width: 4,
            height: 4,
            timestamp: Instant::now(),
            state: AnalyzerStateSnapshot::default(),
        };
        deck.send_frame(&input, &HashMap::new());
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
