use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// ISF shader metadata parsed from JSON header
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ISFMetadata {
    /// Shader description
    #[serde(rename = "DESCRIPTION")]
    pub description: Option<String>,

    /// Shader credit/author
    #[serde(rename = "CREDIT")]
    pub credit: Option<String>,

    /// Categories (e.g., "Generator", "Filter", "Audio Reactive")
    #[serde(rename = "CATEGORIES")]
    pub categories: Option<Vec<String>>,

    /// Input definitions
    #[serde(rename = "INPUTS")]
    pub inputs: Option<Vec<ISFInput>>,

    /// Multi-pass rendering definitions
    #[serde(rename = "PASSES")]
    pub passes: Option<Vec<ISFPass>>,

    /// Imported images/resources
    #[serde(rename = "IMPORTED")]
    pub imported: Option<HashMap<String, ISFImported>>,

    /// Persistent buffers for feedback effects
    #[serde(rename = "PERSISTENT_BUFFERS")]
    pub persistent_buffers: Option<Vec<String>>,

    /// VSN (Version) - ISF spec version
    #[serde(rename = "VSN")]
    pub vsn: Option<String>,

    /// Phase input mappings: which params drive which phase accumulators
    #[serde(rename = "PHASE_INPUTS")]
    pub phase_inputs: Option<Vec<PhaseInput>>,

    /// Preprocessor declarations (analyzers whose outputs are bound as textures/uniforms)
    #[serde(rename = "PREPROCESSORS", default)]
    pub preprocessors: Vec<ISFPreprocessor>,

    /// Shader type: None for fragment shaders, Some("compute") for compute shaders
    #[serde(rename = "TYPE")]
    pub shader_type: Option<String>,

    /// Compute shader configuration (only present for TYPE="compute")
    #[serde(rename = "COMPUTE")]
    pub compute: Option<ComputeConfig>,

    /// Storage buffer declarations (only for compute shaders)
    #[serde(rename = "BUFFERS", default)]
    pub buffers: Vec<StorageBufferDecl>,
}

/// Compute shader dispatch configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeConfig {
    /// Workgroup size [x, y, z]
    #[serde(rename = "WORKGROUP_SIZE")]
    pub workgroup_size: [u32; 3],

    /// Dispatch mode: "resolution" or "custom"
    #[serde(rename = "DISPATCH")]
    pub dispatch: String,

    /// Number of compute passes per frame (default 1).
    /// Each pass dispatches with a different PASSINDEX value (0, 1, ..., num_passes-1).
    /// Non-persistent storage buffers are cleared before pass 0.
    #[serde(rename = "NUM_PASSES", default = "default_num_passes")]
    pub num_passes: u32,
}

fn default_num_passes() -> u32 {
    1
}

/// Storage buffer declaration in compute shader metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageBufferDecl {
    /// Binding name in GLSL source
    #[serde(rename = "NAME")]
    pub name: String,

    /// Buffer type: "storage" (read-write) or "read-only-storage"
    #[serde(rename = "TYPE")]
    pub buffer_type: String,

    /// Struct name in GLSL source (informational)
    #[serde(rename = "STRUCT")]
    pub struct_name: Option<String>,

    /// Number of elements
    #[serde(rename = "COUNT")]
    pub count: u32,

    /// Byte size per element
    #[serde(rename = "STRIDE")]
    pub stride: u32,

    /// Whether buffer persists across frames
    #[serde(rename = "PERSISTENT", default)]
    pub persistent: bool,
}

/// Declares a preprocessor dependency — an analyzer whose texture/uniform outputs
/// the shader wants injected as bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ISFPreprocessor {
    /// Shader-visible name prefix (e.g. "depth" → `depth_depth_map` uniform)
    #[serde(rename = "NAME")]
    pub name: String,

    /// Analyzer type to source from (e.g. "depth_estimate", "edge_detect", "face_detect")
    #[serde(rename = "TYPE")]
    pub preprocessor_type: String,

    /// Options passed to the analyzer when starting it
    #[serde(rename = "OPTIONS", default)]
    pub options: serde_json::Value,
}

/// ISF input definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ISFInput {
    /// Input name (used as uniform variable name)
    #[serde(rename = "NAME")]
    pub name: String,

    /// Input type: "float", "color", "image", "audio", "audioFFT", "bool", "long", "event", "point2D"
    #[serde(rename = "TYPE")]
    pub input_type: String,

    /// Default value (type depends on TYPE)
    #[serde(rename = "DEFAULT")]
    pub default: Option<serde_json::Value>,

    /// Minimum value (for numeric types)
    #[serde(rename = "MIN")]
    pub min: Option<f32>,

    /// Maximum value (for numeric types)
    #[serde(rename = "MAX")]
    pub max: Option<f32>,

    /// Label for UI display
    #[serde(rename = "LABEL")]
    pub label: Option<String>,

    /// Values for "long" (enum) type
    #[serde(rename = "VALUES")]
    pub values: Option<Vec<serde_json::Value>>,

    /// Labels for enum values
    #[serde(rename = "LABELS")]
    pub labels: Option<Vec<String>>,

    /// Identity value (for image inputs)
    #[serde(rename = "IDENTITY")]
    pub identity: Option<bool>,
}

/// ISF pass definition for multi-pass rendering
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ISFPass {
    /// Target buffer name (None for final pass that renders to screen)
    #[serde(rename = "TARGET")]
    pub target: Option<String>,

    /// Persistent flag (buffer persists across frames)
    #[serde(rename = "PERSISTENT")]
    pub persistent: Option<bool>,

    /// Width expression (e.g., "$WIDTH", "$WIDTH/2")
    #[serde(rename = "WIDTH")]
    pub width: Option<String>,

    /// Height expression
    #[serde(rename = "HEIGHT")]
    pub height: Option<String>,

    /// Float flag (use floating-point texture)
    #[serde(rename = "FLOAT")]
    pub float: Option<bool>,
}

/// Phase input mapping: which user parameter drives which phase accumulator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseInput {
    /// Name of the user parameter that drives this accumulator (e.g., "anim_speed")
    #[serde(rename = "PARAM")]
    pub param: String,

    /// Accumulator index (0–3)
    #[serde(rename = "INDEX")]
    pub index: usize,

    /// Constant scale factor applied to `dt * param_value` (default 1.0)
    #[serde(rename = "SCALE", default = "default_scale")]
    pub scale: f32,
}

fn default_scale() -> f32 {
    1.0
}

/// ISF imported resource definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ISFImported {
    /// Path to imported file
    #[serde(rename = "PATH")]
    pub path: Option<String>,

    /// Type of import (e.g., "image")
    #[serde(rename = "TYPE")]
    pub import_type: Option<String>,
}

impl ISFMetadata {
    /// Check if this shader is a generator (no image inputs)
    pub fn is_generator(&self) -> bool {
        if let Some(inputs) = &self.inputs {
            !inputs.iter().any(|input| input.input_type == "image")
        } else {
            true
        }
    }

    /// Check if this shader is a filter (has image inputs)
    pub fn is_filter(&self) -> bool {
        !self.is_generator()
    }

    /// Check if this shader is a transition (has "Transition" category)
    pub fn is_transition(&self) -> bool {
        self.categories
            .as_ref()
            .is_some_and(|cats| cats.iter().any(|c| c.eq_ignore_ascii_case("transition")))
    }

    /// Check if this shader is audio reactive
    pub fn is_audio_reactive(&self) -> bool {
        if let Some(inputs) = &self.inputs {
            inputs
                .iter()
                .any(|input| input.input_type == "audio" || input.input_type == "audioFFT")
        } else {
            false
        }
    }

    /// Check if this shader is a compute shader
    pub fn is_compute(&self) -> bool {
        self.shader_type.as_deref() == Some("compute")
    }

    /// Get all categories as a single string
    pub fn categories_string(&self) -> String {
        self.categories
            .as_ref()
            .map(|cats| cats.join(", "))
            .unwrap_or_else(|| "Uncategorized".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_phase_inputs_from_json() {
        let json = r#"{
            "DESCRIPTION": "Test shader",
            "INPUTS": [],
            "PHASE_INPUTS": [
                {"PARAM": "anim_speed", "INDEX": 0, "SCALE": 0.3},
                {"PARAM": "rot_speed", "INDEX": 1}
            ]
        }"#;
        let meta: ISFMetadata = serde_json::from_str(json).unwrap();
        let pi = meta.phase_inputs.unwrap();
        assert_eq!(pi.len(), 2);
        assert_eq!(pi[0].param, "anim_speed");
        assert_eq!(pi[0].index, 0);
        assert!((pi[0].scale - 0.3).abs() < 1e-5);
        assert_eq!(pi[1].param, "rot_speed");
        assert_eq!(pi[1].index, 1);
        assert!(
            (pi[1].scale - 1.0).abs() < 1e-5,
            "Default scale should be 1.0"
        );
    }

    #[test]
    fn parse_metadata_without_phase_inputs() {
        let json = r#"{
            "DESCRIPTION": "Test shader",
            "INPUTS": []
        }"#;
        let meta: ISFMetadata = serde_json::from_str(json).unwrap();
        assert!(meta.phase_inputs.is_none());
    }

    #[test]
    fn parse_preprocessors() {
        let json = r#"{
            "DESCRIPTION": "test",
            "INPUTS": [],
            "PREPROCESSORS": [
                {
                    "NAME": "depth",
                    "TYPE": "depth_estimate",
                    "OPTIONS": { "resolution": "half" }
                },
                {
                    "NAME": "edges",
                    "TYPE": "edge_detect"
                }
            ]
        }"#;
        let meta: ISFMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(meta.preprocessors.len(), 2);
        assert_eq!(meta.preprocessors[0].name, "depth");
        assert_eq!(meta.preprocessors[0].preprocessor_type, "depth_estimate");
        assert_eq!(meta.preprocessors[1].name, "edges");
        assert_eq!(meta.preprocessors[1].options, serde_json::Value::Null);
    }

    #[test]
    fn parse_without_preprocessors() {
        let json = r#"{"DESCRIPTION": "test", "INPUTS": []}"#;
        let meta: ISFMetadata = serde_json::from_str(json).unwrap();
        assert!(meta.preprocessors.is_empty());
    }

    #[test]
    fn parse_compute_metadata() {
        let json = r#"{
            "DESCRIPTION": "Test compute shader",
            "TYPE": "compute",
            "COMPUTE": {
                "WORKGROUP_SIZE": [16, 16, 1],
                "DISPATCH": "resolution"
            },
            "INPUTS": [
                {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 10.0}
            ],
            "BUFFERS": [
                {
                    "NAME": "particles",
                    "TYPE": "storage",
                    "STRUCT": "Particle",
                    "COUNT": 65536,
                    "STRIDE": 32,
                    "PERSISTENT": true
                }
            ]
        }"#;
        let meta: ISFMetadata = serde_json::from_str(json).unwrap();
        assert!(meta.is_compute());
        let compute = meta.compute.unwrap();
        assert_eq!(compute.workgroup_size, [16, 16, 1]);
        assert_eq!(compute.dispatch, "resolution");
        assert_eq!(meta.buffers.len(), 1);
        assert_eq!(meta.buffers[0].name, "particles");
        assert_eq!(meta.buffers[0].count, 65536);
        assert_eq!(meta.buffers[0].stride, 32);
        assert!(meta.buffers[0].persistent);
    }

    #[test]
    fn parse_fragment_shader_no_compute() {
        let json = r#"{"DESCRIPTION": "Fragment shader", "INPUTS": []}"#;
        let meta: ISFMetadata = serde_json::from_str(json).unwrap();
        assert!(!meta.is_compute());
        assert!(meta.compute.is_none());
        assert!(meta.buffers.is_empty());
    }

    #[test]
    fn parse_compute_no_buffers() {
        let json = r#"{
            "TYPE": "compute",
            "COMPUTE": {"WORKGROUP_SIZE": [8, 8, 1], "DISPATCH": "resolution"},
            "INPUTS": []
        }"#;
        let meta: ISFMetadata = serde_json::from_str(json).unwrap();
        assert!(meta.is_compute());
        assert!(meta.buffers.is_empty());
    }

    #[test]
    fn phase_input_index_range() {
        let json = r#"{
            "INPUTS": [],
            "PHASE_INPUTS": [
                {"PARAM": "a", "INDEX": 0},
                {"PARAM": "b", "INDEX": 1},
                {"PARAM": "c", "INDEX": 2},
                {"PARAM": "d", "INDEX": 3}
            ]
        }"#;
        let meta: ISFMetadata = serde_json::from_str(json).unwrap();
        let pi = meta.phase_inputs.unwrap();
        assert_eq!(pi.len(), 4);
        for (i, p) in pi.iter().enumerate() {
            assert_eq!(p.index, i);
        }
    }
}
