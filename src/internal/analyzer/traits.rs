//! Core types and trait for the analyzer plugin system.
//!
//! An analyzer receives input frames from a deck, processes them (face detection,
//! brightness analysis, etc.), and publishes results as immutable snapshots.
//! Consumers (modulation engine, shader preprocessors) read snapshots lock-free.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::params::ParamValue;

// ── Output definitions ──────────────────────────────────────────────────────

/// Definition of a scalar output an analyzer can produce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ScalarOutputDef {
    /// Output name (e.g. "`face_x`", "brightness").
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
    /// Output name (e.g. "`depth_map`", "`edge_map`").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Texture format as a string key (mapped to `wgpu::TextureFormat` at bind time).
    ///
    /// Examples: `"r8unorm"`, `"r16float"`, `"rg16float"`, `"rgba8unorm"`.
    pub format: String,
}

/// Resolve a [`TextureOutputDef::format`] string to a `wgpu` format.
///
/// `"color_path"` resolves to the compositing colour format so preprocessor
/// outputs that carry colour stay in the unified pipeline
/// (see `/spec/unified-color-pipeline.md`); every other key is an explicit
/// data format that must not be colour-managed.
pub(crate) fn texture_format_from_str(format: &str) -> Option<wgpu::TextureFormat> {
    Some(match format {
        "r8unorm" => wgpu::TextureFormat::R8Unorm,
        "r16float" => wgpu::TextureFormat::R16Float,
        "rg16float" => wgpu::TextureFormat::Rg16Float,
        "rgba8unorm" => wgpu::TextureFormat::Rgba8Unorm,
        "rgba16float" => wgpu::TextureFormat::Rgba16Float,
        // Raw-float data payloads. Not filterable: only legal for shaders
        // that read with `texelFetch`.
        "rgba32float" => wgpu::TextureFormat::Rgba32Float,
        "color_path" => crate::renderer::context::COLOR_PATH_FORMAT,
        _ => return None,
    })
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
    /// Monotonic content generation. Zero asks consumers to upload every
    /// snapshot; non-zero generations may be skipped when already resident.
    pub generation: u64,
    /// Texture width in pixels.
    pub width: u32,
    /// Texture height in pixels.
    pub height: u32,
    /// Format string matching [`TextureOutputDef::format`].
    pub format: String,
    /// Raw pixel data in the specified format.
    pub data: Arc<[u8]>,
}

/// Process-wide texture generation, so restarting an analyzer cannot collide
/// with a generation already resident in its deck slot.
///
/// Part of the preprocessor texture contract, with no in-tree consumer at
/// present: the deck's upload path skips a slot whose generation is unchanged,
/// so any analyzer publishing a texture needs this to have its updates seen.
#[allow(dead_code)]
pub(crate) fn next_texture_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_def(name: &str, default: f32) -> ScalarOutputDef {
        ScalarOutputDef {
            name: name.to_string(),
            description: String::new(),
            range: (0.0, 1.0),
            default,
            default_smoothing: 0.0,
        }
    }

    #[test]
    fn empty_snapshot_has_no_outputs() {
        let snap = AnalyzerSnapshot::empty();
        assert!(snap.scalars.is_empty());
        assert!(snap.textures.is_empty());
    }

    #[test]
    fn from_defaults_with_empty_schema_is_empty() {
        let schema = AnalyzerSchema {
            scalars: vec![],
            textures: vec![],
        };
        let snap = AnalyzerSnapshot::from_defaults(&schema);
        assert!(snap.scalars.is_empty());
    }

    #[test]
    fn from_defaults_populates_each_scalar_default() {
        let schema = AnalyzerSchema {
            scalars: vec![scalar_def("brightness", 0.5), scalar_def("hue", 0.0)],
            textures: vec![],
        };
        let snap = AnalyzerSnapshot::from_defaults(&schema);
        assert_eq!(snap.scalars.len(), 2);
        assert_eq!(snap.scalars.get("brightness"), Some(&0.5));
        assert_eq!(snap.scalars.get("hue"), Some(&0.0));
    }

    #[test]
    fn scalar_returns_value_when_present() {
        let schema = AnalyzerSchema {
            scalars: vec![scalar_def("face_x", 0.73)],
            textures: vec![],
        };
        let snap = AnalyzerSnapshot::from_defaults(&schema);
        assert!((snap.scalar("face_x") - 0.73).abs() < 1e-6);
    }

    #[test]
    fn scalar_falls_back_to_zero_when_missing() {
        let snap = AnalyzerSnapshot::empty();
        assert_eq!(snap.scalar("nonexistent"), 0.0);
    }

    #[test]
    fn scalar_lookup_is_case_sensitive() {
        let schema = AnalyzerSchema {
            scalars: vec![scalar_def("Face_X", 0.5)],
            textures: vec![],
        };
        let snap = AnalyzerSnapshot::from_defaults(&schema);
        assert_eq!(snap.scalar("Face_X"), 0.5);
        assert_eq!(snap.scalar("face_x"), 0.0);
    }
}

// ── Input ────────────────────────────────────────────────────────────────────

/// Immutable live values explicitly bound by one preprocessor declaration.
#[derive(Debug, Clone, Default)]
pub(crate) struct AnalyzerStateSnapshot {
    /// Analyzer-local names mapped to effective shader parameter values or phases.
    pub values: HashMap<String, ParamValue>,
}

impl AnalyzerStateSnapshot {
    /// Typed reads of the bound values. No in-tree analyzer binds parameters
    /// at present, so these are unused; they are the accessor half of the
    /// preprocessor parameter-binding contract and are kept with it.
    #[allow(dead_code)]
    pub(crate) fn float(&self, name: &str) -> Option<f32> {
        match self.values.get(name) {
            Some(ParamValue::Float(value)) => Some(*value),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn long(&self, name: &str) -> Option<i32> {
        match self.values.get(name) {
            Some(ParamValue::Long(value)) => Some(*value),
            _ => None,
        }
    }
}

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
    /// Live parameter and phase values declared by this preprocessor.
    pub state: AnalyzerStateSnapshot,
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

    /// Whether this analyzer reads the deck's pixels.
    ///
    /// Most do, and for those the deck reads its own frame back from the GPU
    /// each frame and hands it over. Some produce output from their options and
    /// parameters alone, and for those the readback is pure cost: it stalls the
    /// pipeline for milliseconds and, because the deck's texture is in the
    /// linear-light colour-path format rather than eight-bit RGBA, it is also
    /// a format mismatch that fails validation.
    ///
    /// Returning `false` means "tick me, but do not read the frame". Such an
    /// analyzer still has `analyze` called on its own thread, with a placeholder
    /// input it is expected to ignore.
    fn needs_frame_input(&self) -> bool {
        true
    }

    /// Analyze a single frame. Called on the analyzer's dedicated thread.
    ///
    /// When [`Self::needs_frame_input`] is `false`, `input` is a placeholder and
    /// its pixels must not be read.
    fn analyze(&mut self, input: &AnalyzerInput) -> anyhow::Result<AnalyzerSnapshot>;

    /// Cleanup when analyzer is stopped. Default is no-op.
    fn shutdown(&mut self) {}
}
