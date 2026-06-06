//! Analyzer plugin system — frame analysis for modulation and shader preprocessing.
//!
//! See `/spec/plugin-architecture.md` for the full design.

pub(crate) mod brightness;
pub(crate) mod face_detect;
pub(crate) mod traits;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, Sender, TrySendError};

use traits::{Analyzer, AnalyzerInput, AnalyzerSchema, AnalyzerSnapshot};

// ── Registry ────────────────────────────────────────────────────────────────

type AnalyzerFactory = Box<dyn Fn() -> Box<dyn Analyzer> + Send + Sync>;

/// Registry of available analyzer types. Built at app startup via builder pattern.
pub(crate) struct AnalyzerRegistry {
    factories: HashMap<String, AnalyzerFactory>,
    schemas: HashMap<String, AnalyzerSchema>,
}

impl AnalyzerRegistry {
    pub(crate) fn new() -> Self {
        Self {
            factories: HashMap::new(),
            schemas: HashMap::new(),
        }
    }

    /// Register an analyzer type with a factory function.
    pub(crate) fn register<F>(mut self, analyzer_type: &str, factory: F) -> Self
    where
        F: Fn() -> Box<dyn Analyzer> + Send + Sync + 'static,
    {
        let instance = factory();
        let schema = instance.output_schema();
        self.schemas.insert(analyzer_type.to_owned(), schema);
        self.factories
            .insert(analyzer_type.to_owned(), Box::new(factory));
        self
    }

    /// Create a new instance of the given analyzer type.
    pub(crate) fn create(&self, analyzer_type: &str) -> Option<Box<dyn Analyzer>> {
        self.factories.get(analyzer_type).map(|f| f())
    }

    /// List all registered analyzer type names.
    pub(crate) fn available_types(&self) -> Vec<&str> {
        self.factories.keys().map(|s| s.as_str()).collect()
    }

    /// Get the output schema for a registered analyzer type.
    pub(crate) fn schema_for(&self, analyzer_type: &str) -> Option<&AnalyzerSchema> {
        self.schemas.get(analyzer_type)
    }
}

// ── Per-Deck Instance Management ────────────────────────────────────────────

struct AnalyzerInstance {
    refcount: usize,
    thread: Option<JoinHandle<()>>,
    latest: Arc<ArcSwap<AnalyzerSnapshot>>,
    stop: Arc<AtomicBool>,
    frame_tx: Sender<AnalyzerInput>,
}

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

        let mut analyzer = registry.create(analyzer_type)?;
        if let Err(e) = analyzer.init(options) {
            log::error!("Failed to init analyzer '{analyzer_type}': {e}");
            return None;
        }

        let schema = analyzer.output_schema();
        let initial = AnalyzerSnapshot::from_defaults(&schema);
        let latest = Arc::new(ArcSwap::from_pointee(initial));
        let stop = Arc::new(AtomicBool::new(false));
        let (frame_tx, frame_rx) = crossbeam_channel::bounded(2);

        let thread_latest = Arc::clone(&latest);
        let thread_stop = Arc::clone(&stop);
        let type_name = analyzer_type.to_owned();

        let thread = std::thread::Builder::new()
            .name(format!("analyzer-{type_name}"))
            .spawn(move || {
                analyzer_thread(analyzer, frame_rx, thread_latest, thread_stop, &type_name);
            })
            .ok()?;

        log::info!("Spawned analyzer '{analyzer_type}'");
        let handle = Arc::clone(&latest);
        self.instances.insert(
            analyzer_type.to_owned(),
            AnalyzerInstance {
                refcount: 1,
                thread: Some(thread),
                latest,
                stop,
                frame_tx,
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

        if should_remove {
            if let Some(mut inst) = self.instances.remove(analyzer_type) {
                inst.stop.store(true, Ordering::Relaxed);
                drop(inst.frame_tx);
                if let Some(thread) = inst.thread.take() {
                    let _ = thread.join();
                }
                log::info!("Stopped analyzer '{analyzer_type}'");
            }
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

    /// Iterate over all active analyzer snapshots: (analyzer_type, snapshot).
    pub(crate) fn all_snapshots(
        &self,
    ) -> impl Iterator<Item = (String, arc_swap::Guard<Arc<AnalyzerSnapshot>>)> + '_ {
        self.instances
            .iter()
            .map(|(k, inst)| (k.clone(), inst.latest.load()))
    }

    /// Send a frame to all running analyzers (non-blocking, drops if full).
    pub(crate) fn send_frame(&self, input: &AnalyzerInput) {
        for (name, inst) in &self.instances {
            match inst.frame_tx.try_send(input.clone()) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    log::warn!("Analyzer '{name}' channel disconnected");
                }
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
    ) -> Option<wgpu::CommandBuffer> {
        if self.instances.is_empty() {
            return None;
        }

        let tex_width = source_texture.width();
        let tex_height = source_texture.height();

        // Create or recreate readback buffer if dimensions changed
        if self.readback.is_none() || self.readback_size != (tex_width, tex_height) {
            self.readback = Some(crate::renderer::ReadbackBuffer::new(
                device, tex_width, tex_height,
            ));
            self.readback_size = (tex_width, tex_height);
        }

        // Read the PREVIOUS frame's data (before mutating readback state)
        let prev_frame = self.readback.as_mut().and_then(|rb| rb.try_read(device));

        // Deliver previous frame data to analyzer threads
        if let Some(rgba_data) = prev_frame {
            let input = AnalyzerInput {
                frame: rgba_data,
                width: self.readback_size.0,
                height: self.readback_size.1,
                timestamp: std::time::Instant::now(),
            };
            self.send_frame(&input);
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
            if let Some(mut inst) = self.instances.remove(&t) {
                inst.stop.store(true, Ordering::Relaxed);
                drop(inst.frame_tx);
                if let Some(thread) = inst.thread.take() {
                    let _ = thread.join();
                }
                log::info!("Stopped analyzer '{t}' (deck shutdown)");
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
    frame_rx: Receiver<AnalyzerInput>,
    latest: Arc<ArcSwap<AnalyzerSnapshot>>,
    stop: Arc<AtomicBool>,
    type_name: &str,
) {
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

// ── Default Registry ────────────────────────────────────────────────────────

/// Build the default analyzer registry with all built-in analyzers.
pub(crate) fn default_registry() -> AnalyzerRegistry {
    AnalyzerRegistry::new()
        .register("brightness", || {
            Box::new(brightness::BrightnessAnalyzer::new())
        })
        .register("face_detect", || {
            Box::new(face_detect::FaceDetectAnalyzer::new())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

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
        };
        deck.send_frame(&input);
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
}
