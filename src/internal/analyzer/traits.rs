//! Core types and trait for the analyzer plugin system.
//!
//! An analyzer receives input frames from a deck, processes them (face detection,
//! brightness analysis, etc.), and publishes results as immutable snapshots.
//! Consumers (modulation engine, shader preprocessors) read snapshots lock-free.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

// ── Output definitions ──────────────────────────────────────────────────────

/// Definition of a scalar output an analyzer can produce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ScalarOutputDef {
    /// Output name (e.g. "face_x", "brightness").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Expected value range, typically `(0.0, 1.0)`.
    pub range: (f32, f32),
    /// Value returned when analysis has no result (e.g. no face detected).
    pub default: f32,
    /// Default smoothing in seconds for modulation consumers.
    pub default_smoothing: f32,
}

/// Definition of a texture output an analyzer can produce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TextureOutputDef {
    /// Output name (e.g. "depth_map", "edge_map").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Texture format as a string key (mapped to `wgpu::TextureFormat` at bind time).
    ///
    /// Examples: `"r8unorm"`, `"r16float"`, `"rg16float"`, `"rgba8unorm"`.
    pub format: String,
}

/// Schema declaring all outputs an analyzer can produce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AnalyzerSchema {
    /// Scalar float outputs (consumed by modulation engine).
    pub scalars: Vec<ScalarOutputDef>,
    /// Texture outputs (consumed by shader preprocessor bindings).
    pub textures: Vec<TextureOutputDef>,
}

// ── Snapshot ─────────────────────────────────────────────────────────────────

/// Raw texture data produced by an analyzer, to be uploaded to GPU by the consumer.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used when runtime texture injection is wired up
pub(crate) struct TextureData {
    /// Texture width in pixels.
    pub width: u32,
    /// Texture height in pixels.
    pub height: u32,
    /// Format string matching [`TextureOutputDef::format`].
    pub format: String,
    /// Raw pixel data in the specified format.
    pub data: Vec<u8>,
}

/// Immutable snapshot of analyzer results, published lock-free via `ArcSwap`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used when runtime texture injection is wired up
pub(crate) struct AnalyzerSnapshot {
    /// Named scalar values (e.g. `"face_x"` → `0.73`).
    pub scalars: HashMap<String, f32>,
    /// Named texture outputs.
    pub textures: HashMap<String, TextureData>,
    /// When this snapshot was produced.
    pub timestamp: Instant,
}

#[allow(dead_code)] // Methods used when runtime texture injection is wired up
impl AnalyzerSnapshot {
    /// Create an empty snapshot (used as initial state before first analysis).
    pub fn empty() -> Self {
        Self {
            scalars: HashMap::new(),
            textures: HashMap::new(),
            timestamp: Instant::now(),
        }
    }

    /// Create a snapshot pre-populated with schema default values for all scalars.
    ///
    /// Pre-allocates the hashmap to avoid rehashing.
    pub fn from_defaults(schema: &AnalyzerSchema) -> Self {
        let mut scalars = HashMap::with_capacity(schema.scalars.len());
        for s in &schema.scalars {
            scalars.insert(s.name.clone(), s.default);
        }
        Self {
            scalars,
            textures: HashMap::new(),
            timestamp: Instant::now(),
        }
    }

    /// Get a scalar value by name, returning `0.0` if not present.
    pub fn scalar(&self, name: &str) -> f32 {
        self.scalars.get(name).copied().unwrap_or(0.0)
    }
}

// ── Input ────────────────────────────────────────────────────────────────────

/// Input frame delivered to an analyzer for processing.
#[derive(Debug, Clone)]
pub(crate) struct AnalyzerInput {
    /// RGBA pixel data, downscaled from the deck's source frame.
    pub frame: Vec<u8>,
    /// Width of the downscaled frame in pixels.
    pub width: u32,
    /// Height of the downscaled frame in pixels.
    pub height: u32,
    /// When the source frame was captured.
    pub timestamp: Instant,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// The core analyzer trait. Implement this to create a new analyzer plugin.
///
/// Analyzers run on dedicated threads and publish results as [`AnalyzerSnapshot`]s.
/// The engine handles threading, lifecycle, and snapshot delivery — implementors
/// only need to define the analysis logic.
pub(crate) trait Analyzer: Send + 'static {
    /// Unique type identifier (e.g. `"face_detect"`, `"brightness"`).
    ///
    /// Must be stable across sessions for serialization.
    #[allow(dead_code)] // Used for logging/serialization when analyzers are active
    fn analyzer_type(&self) -> &str;

    /// Declare all outputs this analyzer can produce.
    fn output_schema(&self) -> AnalyzerSchema;

    /// Initialize with options from the ISF `PREPROCESSORS` block or user config.
    ///
    /// Called once before analysis begins.
    fn init(&mut self, options: &serde_json::Value) -> anyhow::Result<()>;

    /// Analyze a single frame. Called on the analyzer's dedicated thread.
    fn analyze(&mut self, input: &AnalyzerInput) -> anyhow::Result<AnalyzerSnapshot>;

    /// Cleanup when analyzer is stopped. Default is no-op.
    fn shutdown(&mut self) {}
}
