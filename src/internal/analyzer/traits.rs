//! Core types and trait for analyzers.
//!
//! An analyzer takes frames from a deck, processes them (face detection,
//! brightness, etc.), and publishes immutable snapshots. The modulation engine
//! and shader preprocessors read snapshots lock-free.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::params::ParamValue;

// ── Output definitions ──────────────────────────────────────────────────────

/// A scalar output an analyzer can produce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ScalarOutputDef {
    /// e.g. "`face_x`", "brightness".
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Expected value range, typically `(0.0, 1.0)`.
    pub range: (f32, f32),
    /// Value when analysis has no result (e.g. no face detected).
    pub default: f32,
    /// Default modulation smoothing in seconds.
    pub default_smoothing: f32,
}

/// A texture output an analyzer can produce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TextureOutputDef {
    /// e.g. "`depth_map`", "`edge_map`".
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Format key, mapped to `wgpu::TextureFormat` at bind time, e.g.
    /// `"r8unorm"`, `"r16float"`, `"rg16float"`, `"rgba8unorm"`.
    pub format: String,
}

/// Resolves a [`TextureOutputDef::format`] string to a `wgpu` format.
/// `"color_path"` maps to the compositing color format for outputs that carry
/// color; every other key is a data format that is not color-managed.
pub(crate) fn texture_format_from_str(format: &str) -> Option<wgpu::TextureFormat> {
    Some(match format {
        "r8unorm" => wgpu::TextureFormat::R8Unorm,
        "r16float" => wgpu::TextureFormat::R16Float,
        "rg16float" => wgpu::TextureFormat::Rg16Float,
        "rgba8unorm" => wgpu::TextureFormat::Rgba8Unorm,
        "rgba16float" => wgpu::TextureFormat::Rgba16Float,
        // Raw-float data, not filterable: shaders must read it with `texelFetch`.
        "rgba32float" => wgpu::TextureFormat::Rgba32Float,
        "color_path" => crate::renderer::context::COLOR_PATH_FORMAT,
        _ => return None,
    })
}

/// Every output an analyzer can produce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AnalyzerSchema {
    /// Read by the modulation engine.
    pub scalars: Vec<ScalarOutputDef>,
    /// Read by shader preprocessor bindings.
    pub textures: Vec<TextureOutputDef>,
}

// ── Snapshot ─────────────────────────────────────────────────────────────────

/// Raw texture data from an analyzer, for the consumer to upload.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used when runtime texture injection is wired up
pub(crate) struct TextureData {
    /// Monotonic content generation. Zero means upload every snapshot; a
    /// non-zero generation already resident may be skipped.
    pub generation: u64,
    /// Pixels.
    pub width: u32,
    /// Pixels.
    pub height: u32,
    /// Format string matching [`TextureOutputDef::format`].
    pub format: String,
    /// Pixel data in `format`.
    pub data: Arc<[u8]>,
}

/// Process-wide texture generation, so a restarted analyzer can't reuse a
/// generation already resident in its deck slot. Unused in-tree, but the
/// deck skips uploads whose generation is unchanged, so any analyzer that
/// publishes textures needs it.
#[allow(dead_code)]
pub(crate) fn next_texture_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Immutable analyzer results, published lock-free via `ArcSwap`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used when runtime texture injection is wired up
pub(crate) struct AnalyzerSnapshot {
    /// e.g. `"face_x"` → `0.73`.
    pub scalars: HashMap<String, f32>,
    pub textures: HashMap<String, TextureData>,
    /// When this snapshot was produced.
    pub timestamp: Instant,
}

#[allow(dead_code)] // Methods used when runtime texture injection is wired up
impl AnalyzerSnapshot {
    /// Empty snapshot, the state before the first analysis.
    pub fn empty() -> Self {
        Self {
            scalars: HashMap::new(),
            textures: HashMap::new(),
            timestamp: Instant::now(),
        }
    }

    /// Snapshot with every scalar set to its schema default.
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

    /// Scalar by name, or `0.0` if missing.
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

/// Live values bound by one preprocessor declaration.
#[derive(Debug, Clone, Default)]
pub(crate) struct AnalyzerStateSnapshot {
    /// Analyzer-local names mapped to shader parameter values or phases.
    pub values: HashMap<String, ParamValue>,
}

impl AnalyzerStateSnapshot {
    /// Typed reads of the bound values. Unused in-tree, since no analyzer
    /// binds parameters yet; kept as part of the binding API.
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

/// Input frame for an analyzer.
#[derive(Debug, Clone)]
pub(crate) struct AnalyzerInput {
    /// Display-encoded RGBA8 pixels, reduced from the deck's frame and shared
    /// by every analyzer on the deck. Empty for analyzers that read no pixels.
    pub frame: Arc<Vec<u8>>,
    /// Pixels.
    pub width: u32,
    /// Pixels.
    pub height: u32,
    /// When the source frame was captured.
    pub timestamp: Instant,
    /// Live parameter and phase values bound by this preprocessor.
    pub state: AnalyzerStateSnapshot,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// An analyzer. Runs on its own thread and publishes [`AnalyzerSnapshot`]s; the
/// engine handles threading, lifecycle, and delivery.
pub(crate) trait Analyzer: Send + 'static {
    /// Stable type id (e.g. `"face_detect"`, `"brightness"`), used in
    /// serialization.
    #[allow(dead_code)] // Used for logging/serialization when analyzers are active
    fn analyzer_type(&self) -> &str;

    /// Every output this analyzer can produce.
    fn output_schema(&self) -> AnalyzerSchema;

    /// Called once before analysis, with options from the ISF `PREPROCESSORS`
    /// block or user config.
    fn init(&mut self, options: &serde_json::Value) -> anyhow::Result<()>;

    /// Long side in pixels of the frame this analyzer reads, or `None` if it
    /// reads no pixels.
    ///
    /// The deck reduces its frame on the GPU to the largest size its analyzers
    /// ask for, never above its own. With `None`, `analyze` still runs, with a
    /// placeholder input that has the deck's size and no pixels.
    fn frame_size(&self) -> Option<u32>;

    /// Analyzes one frame, on the analyzer's thread. When
    /// [`Self::frame_size`] is `None`, `input` is a placeholder and its pixels
    /// must not be read. Read the size from `input`; it may be smaller than
    /// asked for.
    fn analyze(&mut self, input: &AnalyzerInput) -> anyhow::Result<AnalyzerSnapshot>;

    /// Called when the analyzer stops. Default does nothing.
    fn shutdown(&mut self) {}
}

/// One rendered frame, as a host-inline preprocessor sees it.
#[derive(Debug, Clone)]
pub(crate) struct HostFrame<'a> {
    /// Seconds the deck's previous frame took on the wall clock, when frames
    /// are paced by it. Offline renders pass their fixed step (`TIMEDELTA`),
    /// so their output does not depend on the machine.
    pub frame_seconds: f32,
    /// Live parameter and phase values bound by `PARAM_BINDINGS` and
    /// `PHASE_BINDINGS`.
    pub state: &'a AnalyzerStateSnapshot,
}

/// A preprocessor stepped on the render thread once per rendered frame, before
/// the deck's shader, so its outputs always belong to the frame being drawn.
///
/// Instances belong to one deck. Expensive, latency-tolerant work goes to a
/// worker the preprocessor owns; `step` must stay within its registered
/// budget.
pub(crate) trait HostInlinePreprocessor: Send + 'static {
    /// Every output this preprocessor can produce.
    fn output_schema(&self) -> AnalyzerSchema;

    /// Called once before the first step, with the ISF `OPTIONS`.
    fn init(&mut self, options: &serde_json::Value) -> anyhow::Result<()>;

    /// Advance one frame and return this frame's outputs.
    fn step(&mut self, frame: &HostFrame<'_>) -> AnalyzerSnapshot;

    /// State that must survive a save, or `None` if there is none.
    fn persisted_state(&self) -> Option<serde_json::Value> {
        None
    }

    /// Replace the state with a saved one. May be called before or after
    /// `init`.
    ///
    /// # Errors
    ///
    /// When the value is not a state this preprocessor wrote. The caller logs
    /// it and keeps the current state.
    fn restore_state(&mut self, _state: &serde_json::Value) -> anyhow::Result<()> {
        Ok(())
    }

    /// Something to tell the performer once. Taking it clears it, so one
    /// event gives one toast.
    fn take_message(&mut self) -> Option<String> {
        None
    }

    /// Called when the deck drops the preprocessor.
    fn shutdown(&mut self) {}
}
