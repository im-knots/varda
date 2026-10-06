//! Analyzer-driven texture slots a shader source binds.

use std::collections::HashMap;

/// A preprocessor texture slot, updated with analyzer output.
pub struct PreprocessorSlot {
    /// Name prefix for shader uniforms (e.g. "depth" → `depth_depth_map`)
    pub name: String,
    /// Analyzer type this preprocessor needs (e.g. "`depth_estimate`")
    pub analyzer_type: String,
    /// Options passed when starting the analyzer.
    pub options: serde_json::Value,
    /// Analyzer value name to live shader parameter name.
    pub param_bindings: HashMap<String, String>,
    /// Analyzer value name to phase-accumulator index.
    pub phase_bindings: HashMap<String, usize>,
    /// Inputs set each frame from the scalar outputs of the same name.
    pub writes: Vec<String>,
    /// Starts as 1×1 black.
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Fixed for the slot's lifetime: the pipeline layout's filterability
    /// derives from it.
    pub format: wgpu::TextureFormat,
    /// Last non-zero analyzer texture generation uploaded to this slot.
    pub last_uploaded_generation: Option<u64>,
}

/// The texture format a `PREPROCESSORS` entry's `FORMAT` string names.
///
/// `rgba32float` binds non-filterable, for `texelFetch`-only float data. Any
/// other value gives the filterable byte format.
pub fn preprocessor_texture_format(declared: &str) -> wgpu::TextureFormat {
    crate::analyzer::traits::texture_format_from_str(declared)
        .unwrap_or(wgpu::TextureFormat::Rgba8Unorm)
}
