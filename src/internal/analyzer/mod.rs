//! Analyzer plugin system — frame analysis for modulation and shader preprocessing.
//!
//! See `/spec/plugin-architecture.md` for the full design.

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

/// Execution category of a preprocessor type.
///
/// Shaders declare all categories identically in their ISF `PREPROCESSORS` block;
/// the category tells the engine how to run the thing and whether a shader that
/// declares it can load without it. See `/spec/effect-preprocessing.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreprocessorCategory {
    /// Worker thread consuming a CPU readback of the deck's own frame, publishing
    /// `AnalyzerSnapshot`s. Optional — degrades to default outputs.
    CpuAnalyzer,
    /// GPU passes reading textures owned by an external device manager. The device
    /// is acquired at load time; if it is unavailable the shader does not load.
    GpuDeviceBacked,
    // A third category — GPU passes reading the deck's own frame (`GpuFrameDerived`,
    // e.g. edge detect) — is designed in /spec/effect-preprocessing.md but not yet
    // implemented; it needs a frame-input path no current preprocessor uses.
}

impl PreprocessorCategory {
    /// Whether this category runs as GPU passes rather than an analyzer thread.
    /// GPU categories have no factory and must never be handed to `DeckAnalyzers`.
    pub(crate) fn is_gpu(self) -> bool {
        !matches!(self, Self::CpuAnalyzer)
    }

    /// Whether a shader declaring this type must fail to load when it is unavailable.
    pub(crate) fn is_required(self) -> bool {
        matches!(self, Self::GpuDeviceBacked)
    }
}

/// Registry of available preprocessor types. Built at app startup via builder pattern.
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

    /// Register a CPU-async analyzer type with a factory function.
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

    /// Register a GPU-inline preprocessor type. These have no factory and no worker
    /// thread — the deck render path owns their passes — so the schema is supplied
    /// directly rather than read off an instance.
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

    /// Create a new instance of the given analyzer type. Returns `None` for GPU
    /// categories, which have no factory by construction.
    pub(crate) fn create(&self, analyzer_type: &str) -> Option<Box<dyn Analyzer>> {
        self.factories.get(analyzer_type).map(|f| f())
    }

    /// List all registered preprocessor type names, GPU and CPU alike.
    pub(crate) fn available_types(&self) -> Vec<&str> {
        self.categories.keys().map(String::as_str).collect()
    }

    /// Get the output schema for a registered preprocessor type.
    pub(crate) fn schema_for(&self, analyzer_type: &str) -> Option<&AnalyzerSchema> {
        self.schemas.get(analyzer_type)
    }

    /// Get the execution category for a registered preprocessor type.
    pub(crate) fn category_for(&self, preprocessor_type: &str) -> Option<PreprocessorCategory> {
        self.categories.get(preprocessor_type).copied()
    }
}

// ── Per-Deck Instance Management ────────────────────────────────────────────

struct AnalyzerInstance {
    refcount: usize,
    /// Whether this analyzer reads the deck's pixels. Captured at creation
    /// because the analyzer itself moves onto its worker thread.
    needs_frames: bool,
    thread: Option<JoinHandle<()>>,
    latest: Arc<ArcSwap<AnalyzerSnapshot>>,
    stop: Arc<AtomicBool>,
    frame_tx: Sender<AnalyzerInput>,
    /// Disconnects when the worker thread exits (the matching sender is owned by
    /// the thread). Used for a bounded, non-blocking stop on shutdown.
    done_rx: Receiver<()>,
}

/// Grace period to wait for an analyzer worker to exit before detaching it, so a
/// thread wedged in a blocking FFI call (e.g. ONNX Runtime) can never freeze
/// application shutdown.
const STOP_GRACE: Duration = Duration::from_secs(2);

/// Manages running analyzer instances for a single deck.
pub(crate) struct DeckAnalyzers {
    instances: HashMap<String, AnalyzerInstance>,
    /// Lazy GPU readback buffer — created on first `capture_frame` call.
    readback: Option<crate::renderer::ReadbackBuffer>,
    /// Cached dimensions of the current readback buffer.
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

    /// Request an analyzer type. If already running, increments refcount.
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

        // Schema is static and does not require init(), so we can build the
        // initial default snapshot before the worker thread runs.
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
        // init() can be expensive (e.g. loading + optimizing ONNX models), so it
        // runs inside the worker thread to keep the UI/render thread responsive.
        let options = options.clone();

        let thread = std::thread::Builder::new()
            .name(format!("analyzer-{type_name}"))
            .spawn(move || {
                // Dropped when the thread exits (normally or via panic),
                // disconnecting `done_rx` so stoppers can wait with a timeout.
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

    /// Release an analyzer reference. Stops when refcount reaches zero.
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

    /// Get the latest snapshot for a specific analyzer type.
    pub(crate) fn latest_snapshot(
        &self,
        analyzer_type: &str,
    ) -> Option<arc_swap::Guard<Arc<AnalyzerSnapshot>>> {
        self.instances
            .get(analyzer_type)
            .map(|inst| inst.latest.load())
    }

    /// Iterate over all active analyzer snapshots: (`analyzer_type`, snapshot).
    pub(crate) fn all_snapshots(
        &self,
    ) -> impl Iterator<Item = (String, arc_swap::Guard<Arc<AnalyzerSnapshot>>)> + '_ {
        self.instances
            .iter()
            .map(|(k, inst)| (k.clone(), inst.latest.load()))
    }

    /// Send a frame to all running analyzers (non-blocking, drops if full).
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

    /// Remove instances whose worker thread has exited (e.g. `init()` failed
    /// because a dependency like ONNX Runtime is unavailable). Without this a
    /// dead instance lingers in the map, causing the render loop to perform a
    /// per-frame GPU readback and spam "channel disconnected" warnings on the
    /// hot path. Pruning is cheap: a non-blocking `try_recv` on the rendezvous
    /// channel that disconnects when the worker drops its sender.
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

    /// Capture the current deck texture for analysis and deliver previous frame's data to analyzers.
    /// Call this from the render loop after effects are applied.
    /// Returns a command buffer with the readback copy command, or None if no analyzers are active.
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

        // Analyzers that produce output from their options alone do not want the
        // frame, and reading it for them is worse than wasteful. The readback
        // stalls the pipeline for milliseconds, and it assumes eight-bit RGBA
        // while a deck's texture is in the linear-light colour-path format, so
        // the copy fails validation. That error is contained by the deck rather
        // than raised, and the visible result is a black frame with no message,
        // which is how this went unnoticed: no shader had ever attached a
        // frameless CPU analyzer to a float deck.
        //
        // They still receive source dimensions because geometry-only analyzers
        // may certify camera packets against the render aspect without reading
        // pixels.
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

        // Read back in the texture's own format.
        //
        // This used to be hard-coded to eight-bit RGBA while a deck's texture
        // is `COLOR_PATH_FORMAT`, four half-floats, so the copy asked for half
        // the bytes a row actually holds and wgpu rejected the encoder with
        // "number of bytes per row is less than the number of bytes in a
        // complete row". The whole deck was then quarantined, which is how a
        // camera with an analyzer-backed effect on it died outright.
        let Some(readback_format) = readback_format_for(source_texture.format()) else {
            log::warn!(
                "no analyzer readback for texture format {:?}; frame-consuming \
                 analyzers on this deck will not receive pixels",
                source_texture.format()
            );
            return None;
        };

        // Create or recreate readback buffer if dimensions or format changed
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

        // Read the PREVIOUS frame's data (before mutating readback state)
        let prev_frame = self.readback.as_mut().and_then(|rb| rb.try_read(device));

        // Deliver previous frame data to analyzer threads
        if let Some(rgba_data) = prev_frame {
            let input = AnalyzerInput {
                // Analyzers are promised eight-bit RGBA whatever the deck's
                // own format is.
                frame: frame_to_rgba8(&rgba_data),
                width: self.readback_size.0,
                height: self.readback_size.1,
                timestamp: std::time::Instant::now(),
                state: AnalyzerStateSnapshot::default(),
            };
            for (name, inst) in &self.instances {
                // A frameless analyzer sharing a deck with a frame-consuming one
                // is still ticked, but with nothing it might mistake for pixels.
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

        // Enqueue copy for THIS frame (will be read next frame)
        let readback = self.readback.as_mut().unwrap();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Analyzer readback"),
        });
        readback.begin_readback(&mut encoder, source_texture);
        Some(encoder.finish())
    }

    /// Check if any analyzer instances are currently running.
    pub(crate) fn has_active_instances(&self) -> bool {
        !self.instances.is_empty()
    }

    pub(crate) fn running_types(&self) -> Vec<String> {
        self.instances.keys().cloned().collect()
    }

    /// Stop all running instances.
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
    // Run potentially-expensive initialization off the caller's thread. On
    // failure the thread exits and the deck keeps the default snapshot.
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

/// Stop a single analyzer instance with a bounded wait. Signals the worker to
/// stop and waits up to [`STOP_GRACE`] for it to exit; if it does, the handle is
/// joined (reaped). If it does not — e.g. the thread is wedged in a blocking FFI
/// call — the handle is detached so application shutdown is never frozen.
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
        // Detach the wedged thread; the OS reclaims it on process exit.
        let _ = inst.thread.take();
        log::warn!("Analyzer '{type_name}'{suffix} did not stop within {STOP_GRACE:?}; detaching");
    }
}

// ── Default Registry ────────────────────────────────────────────────────────

/// The readback format that matches a deck texture, if analyzers can read it.
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

/// One linear-light channel as an eight-bit sRGB sample.
///
/// The colour path is linear, and analyzers were written against the
/// display-encoded frame a deck used to hand them, so handing them linear
/// values unchanged would silently darken every brightness and face result.
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

/// Convert a readback frame to the eight-bit RGBA the analyzer contract states.
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
                    // Alpha is already display-linear; only colour is encoded.
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
        // Video layouts never reach a deck texture; the format gate above
        // refuses them before a buffer is ever built.
        R::Uyvy | R::P216 => Vec::new(),
    }
}

/// Build the default analyzer registry with all built-in analyzers.
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
    // Device-backed GPU preprocessor: no factory, no worker thread. Registered
    // unconditionally — without the `depth` feature no sensor enumerates, so a
    // shader declaring it fails its pre-flight with a clear message rather than
    // an "unknown preprocessor type". See /spec/depth-sensor-preprocessor.md.
    registry = registry.register_gpu(
        crate::depth::preprocess::PREPROCESSOR_TYPE,
        PreprocessorCategory::GpuDeviceBacked,
        crate::depth::preprocess::schema(),
    );
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// A frame-consuming analyzer on a colour-path deck must encode a legal copy.
    ///
    /// The readback asked for eight-bit rows while the deck texture holds four
    /// half-floats, so wgpu rejected the encoder and quarantined the whole
    /// deck: a camera with an analyzer-backed effect on it went black with
    /// "number of bytes per row is less than the number of bytes in a complete
    /// row". Submitting the encoder is the assertion, because that is where
    /// validation runs.
    #[test]
    fn colour_path_deck_readback_encodes_a_legal_copy() {
        let Ok(context) = crate::renderer::context::GpuContext::new_headless() else {
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
        // Linear light is display-encoded on the way out: mid-grey in linear is
        // well above mid-grey once encoded, and analyzers were written against
        // the encoded frame.
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
        // Force face_detect's init() to fail by pointing it at a missing model
        // file. The worker thread then exits; the instance must be pruned so the
        // render loop stops per-frame GPU readback and "channel disconnected"
        // log spam. Shutdown must also stay fast (the worker already exited).
        let registry = default_registry();
        let mut deck = DeckAnalyzers::new();
        let opts = serde_json::json!({
            "model_path": "/nonexistent/__varda_missing_model__.onnx"
        });
        let _handle = deck
            .request("face_detect", &registry, &opts)
            .expect("should spawn worker");

        // Poll for the worker to run init() (which fails) and exit, then be
        // pruned. A single fixed sleep is racy: under CPU load the worker may not
        // have finished the failing ONNX init() and dropped its done_tx yet.
        // Poll prune_dead() until the dead instance is removed, bounded by a
        // generous timeout so a genuine hang still fails the test.
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
