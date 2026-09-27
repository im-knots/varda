//! Analyzer-driven texture slots a shader source binds.

use std::collections::HashMap;

/// A preprocessor texture slot — holds a GPU texture that gets updated with analyzer output.
pub struct PreprocessorSlot {
    /// Name prefix for shader uniforms (e.g. "depth" → `depth_depth_map`)
    pub name: String,
    /// Analyzer type this preprocessor needs (e.g. "`depth_estimate`")
    pub analyzer_type: String,
    /// Options to pass when starting the analyzer
    pub options: serde_json::Value,
    /// Analyzer value name to live shader parameter name.
    pub param_bindings: HashMap<String, String>,
    /// Analyzer value name to phase-accumulator index.
    pub phase_bindings: HashMap<String, usize>,
    /// GPU texture (initially 1×1 black, updated at runtime)
    pub texture: wgpu::Texture,
    /// Texture view for shader binding
    pub view: wgpu::TextureView,
    /// Format the shader declared for this slot, fixed for the slot's lifetime
    /// because the pipeline layout's filterability is derived from it.
    pub format: wgpu::TextureFormat,
    /// Last non-zero analyzer texture generation uploaded to this slot.
    pub last_uploaded_generation: Option<u64>,
}

/// The texture format a `PREPROCESSORS` entry's `FORMAT` string names.
///
/// `rgba32float` binds non-filterable and carries four raw floats per texel
/// for `texelFetch`-only data payloads; an unknown or absent declaration is
/// the filterable byte-packed default.
pub fn preprocessor_texture_format(declared: &str) -> wgpu::TextureFormat {
    crate::analyzer::traits::texture_format_from_str(declared)
        .unwrap_or(wgpu::TextureFormat::Rgba8Unorm)
}
