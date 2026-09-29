use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// ISF shader metadata from the JSON header.
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

    /// ISF spec version.
    #[serde(rename = "VSN")]
    pub vsn: Option<String>,

    /// Which params drive which phase accumulators.
    #[serde(rename = "PHASE_INPUTS")]
    pub phase_inputs: Option<Vec<PhaseInput>>,

    /// Analyzers whose outputs are bound as textures/uniforms.
    #[serde(rename = "PREPROCESSORS", default)]
    pub preprocessors: Vec<ISFPreprocessor>,

    /// `None` for fragment shaders, `Some("compute")` for compute shaders.
    #[serde(rename = "TYPE")]
    pub shader_type: Option<String>,

    /// Only present for `TYPE="compute"`.
    #[serde(rename = "COMPUTE")]
    pub compute: Option<ComputeConfig>,

    /// Compute shaders only.
    #[serde(rename = "BUFFERS", default)]
    pub buffers: Vec<StorageBufferDecl>,
}

/// Compute shader dispatch config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeConfig {
    /// Workgroup size [x, y, z]
    #[serde(rename = "WORKGROUP_SIZE")]
    pub workgroup_size: [u32; 3],

    /// Dispatch mode: "resolution" or "custom"
    #[serde(rename = "DISPATCH")]
    pub dispatch: String,

    /// Compute passes per frame (default 1). Each pass gets its own PASSINDEX
    /// (`0..num_passes`). Non-persistent storage buffers are cleared before pass 0.
    #[serde(rename = "NUM_PASSES", default = "default_num_passes")]
    pub num_passes: u32,
}

fn default_num_passes() -> u32 {
    1
}

/// Storage buffer declaration for a compute shader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageBufferDecl {
    /// Binding name in GLSL source
    #[serde(rename = "NAME")]
    pub name: String,

    /// Buffer type: "storage" (read-write) or "read-only-storage"
    #[serde(rename = "TYPE")]
    pub buffer_type: String,

    /// Struct name in GLSL source (informational).
    #[serde(rename = "STRUCT")]
    pub struct_name: Option<String>,

    /// Number of elements
    #[serde(rename = "COUNT")]
    pub count: u32,

    /// Bytes per element.
    #[serde(rename = "STRIDE")]
    pub stride: u32,

    /// Whether the buffer persists across frames.
    #[serde(rename = "PERSISTENT", default)]
    pub persistent: bool,
}

/// A preprocessor dependency: an analyzer whose texture/uniform outputs are
/// injected into the shader as bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ISFPreprocessor {
    /// Shader-visible name prefix (e.g. "depth" → `depth_depth_map` uniform).
    #[serde(rename = "NAME")]
    pub name: String,

    /// Analyzer type (e.g. "`depth_estimate`", "`edge_detect`", "`face_detect`").
    #[serde(rename = "TYPE")]
    pub preprocessor_type: String,

    /// Options passed to the analyzer on start.
    #[serde(rename = "OPTIONS", default)]
    pub options: serde_json::Value,

    /// Analyzer value name to live shader parameter name.
    #[serde(rename = "PARAM_BINDINGS", default)]
    pub param_bindings: HashMap<String, String>,

    /// Analyzer value name to engine phase-accumulator index.
    #[serde(rename = "PHASE_BINDINGS", default)]
    pub phase_bindings: HashMap<String, usize>,

    /// Texture format of this preprocessor's payload.
    ///
    /// `"rgba8unorm"` (default) is filterable. `"rgba32float"` is a
    /// non-filterable data texture (one `texelFetch` returns four raw floats)
    /// and works only if the shader reads it with `texelFetch`/`textureSize`,
    /// never `texture()`.
    #[serde(rename = "FORMAT", default = "default_preprocessor_format")]
    pub format: String,
}

fn default_preprocessor_format() -> String {
    "rgba8unorm".into()
}

/// ISF input definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ISFInput {
    /// Also the uniform variable name.
    #[serde(rename = "NAME")]
    pub name: String,

    /// "float", "color", "image", "audio", "audioFFT", "bool", "long", "event", or "point2D".
    #[serde(rename = "TYPE")]
    pub input_type: String,

    /// Type depends on `TYPE`.
    #[serde(rename = "DEFAULT")]
    pub default: Option<serde_json::Value>,

    /// Numeric types only.
    #[serde(rename = "MIN")]
    pub min: Option<f32>,

    /// Numeric types only.
    #[serde(rename = "MAX")]
    pub max: Option<f32>,

    /// UI label.
    #[serde(rename = "LABEL")]
    pub label: Option<String>,

    /// Values for the "long" (enum) type.
    #[serde(rename = "VALUES")]
    pub values: Option<Vec<serde_json::Value>>,

    /// Labels for enum values.
    #[serde(rename = "LABELS")]
    pub labels: Option<Vec<String>>,

    /// Image inputs only.
    #[serde(rename = "IDENTITY")]
    pub identity: Option<bool>,

    /// Inspector section, a Varda extension. Inputs without one form a single
    /// unnamed group shown first.
    #[serde(rename = "GROUP")]
    pub group: Option<String>,
}

impl ISFInput {
    /// The options of a `long` input, as `(value, label)` pairs, so `VALUES` and
    /// `LABELS` can't disagree on length. `LABELS` alone works (index becomes
    /// value); a value without a label shows its integer. Empty for other types.
    pub fn choices(&self) -> Vec<(i32, String)> {
        if self.input_type != "long" {
            return Vec::new();
        }
        let count = match (self.values.as_ref(), self.labels.as_ref()) {
            (Some(values), _) => values.len(),
            (None, Some(labels)) => labels.len(),
            (None, None) => return Vec::new(),
        };
        (0..count)
            .map(|i| {
                let index = i32::try_from(i).unwrap_or(i32::MAX);
                let value = self
                    .values
                    .as_ref()
                    .and_then(|v| v.get(i))
                    .and_then(serde_json::Value::as_i64)
                    .map_or(index, |n| n as i32);
                let label = self
                    .labels
                    .as_ref()
                    .and_then(|l| l.get(i))
                    .cloned()
                    .unwrap_or_else(|| value.to_string());
                (value, label)
            })
            .collect()
    }
}

/// Pass definition for multi-pass rendering.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ISFPass {
    /// Target buffer name; `None` for the final pass to the screen.
    #[serde(rename = "TARGET")]
    pub target: Option<String>,

    /// Buffer persists across frames.
    #[serde(rename = "PERSISTENT")]
    pub persistent: Option<bool>,

    /// Width expression (e.g. "$WIDTH", "$WIDTH/2").
    #[serde(rename = "WIDTH")]
    pub width: Option<String>,

    /// Height expression.
    #[serde(rename = "HEIGHT")]
    pub height: Option<String>,

    /// Use a floating-point texture.
    #[serde(rename = "FLOAT")]
    pub float: Option<bool>,
}

/// Which user parameter drives which phase accumulator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseInput {
    /// User parameter driving this accumulator (e.g. "`anim_speed`").
    #[serde(rename = "PARAM")]
    pub param: String,

    /// Accumulator index (0–3).
    #[serde(rename = "INDEX")]
    pub index: usize,

    /// Scale applied to `dt * param_value` (default 1.0).
    #[serde(rename = "SCALE", default = "default_scale")]
    pub scale: f32,

    /// Extra parameters multiplied into the rate before integration, for
    /// "master speed × per-element rate". A string or an array of names; absent
    /// means none.
    #[serde(
        rename = "MULTIPLY_BY",
        default,
        deserialize_with = "deserialize_multiply_by",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub multiply_by: Vec<String>,
}

/// Accepts `"MULTIPLY_BY": "rot_speed"` and `"MULTIPLY_BY": ["a", "b"]`.
fn deserialize_multiply_by<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }

    Ok(match Option::<OneOrMany>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(OneOrMany::One(name)) => vec![name],
        Some(OneOrMany::Many(names)) => names,
    })
}

fn default_scale() -> f32 {
    1.0
}

/// Imported resource definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ISFImported {
    #[serde(rename = "PATH")]
    pub path: Option<String>,

    /// Import type (e.g. "image").
    #[serde(rename = "TYPE")]
    pub import_type: Option<String>,
}

impl ISFMetadata {
    /// Generator: no image inputs.
    pub fn is_generator(&self) -> bool {
        if let Some(inputs) = &self.inputs {
            !inputs.iter().any(|input| input.input_type == "image")
        } else {
            true
        }
    }

    /// Filter: has image inputs.
    pub fn is_filter(&self) -> bool {
        !self.is_generator()
    }

    /// Transition: has the "Transition" category.
    pub fn is_transition(&self) -> bool {
        self.categories
            .as_ref()
            .is_some_and(|cats| cats.iter().any(|c| c.eq_ignore_ascii_case("transition")))
    }

    pub fn is_audio_reactive(&self) -> bool {
        if let Some(inputs) = &self.inputs {
            inputs
                .iter()
                .any(|input| input.input_type == "audio" || input.input_type == "audioFFT")
        } else {
            false
        }
    }

    pub fn is_compute(&self) -> bool {
        self.shader_type.as_deref() == Some("compute")
    }

    /// All categories as one string.
    pub fn categories_string(&self) -> String {
        self.categories
            .as_ref()
            .map_or_else(|| "Uncategorized".to_string(), |cats| cats.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_input(values: Option<&str>, labels: Option<&str>) -> ISFInput {
        let json = format!(
            r#"{{"NAME": "mode", "TYPE": "long"{}{}}}"#,
            values.map_or(String::new(), |v| format!(r#", "VALUES": {v}"#)),
            labels.map_or(String::new(), |l| format!(r#", "LABELS": {l}"#)),
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn group_is_optional_and_absent_by_default() {
        let json = r#"{
            "INPUTS": [
                {"NAME": "speed", "TYPE": "float"},
                {"NAME": "fold", "TYPE": "float", "GROUP": "Formula"}
            ]
        }"#;
        let meta: ISFMetadata = serde_json::from_str(json).unwrap();
        let inputs = meta.inputs.unwrap();
        assert!(
            inputs[0].group.is_none(),
            "an input without GROUP must stay ungrouped, which is how every \
             community ISF shader renders"
        );
        assert_eq!(inputs[1].group.as_deref(), Some("Formula"));
    }

    #[test]
    fn choices_pairs_values_with_labels() {
        let input = long_input(Some("[0, 1, 2]"), Some(r#"["Off", "Soft", "Hard"]"#));
        assert_eq!(
            input.choices(),
            vec![
                (0, "Off".to_string()),
                (1, "Soft".to_string()),
                (2, "Hard".to_string()),
            ]
        );
    }

    #[test]
    fn choices_uses_declared_values_not_indices() {
        let input = long_input(Some("[10, 20]"), Some(r#"["Ten", "Twenty"]"#));
        assert_eq!(
            input.choices(),
            vec![(10, "Ten".to_string()), (20, "Twenty".to_string())],
            "the GPU receives VALUES, so the combo must select from them"
        );
    }

    #[test]
    fn choices_falls_back_to_the_integer_when_a_label_is_missing() {
        let input = long_input(Some("[0, 1, 2]"), Some(r#"["Off"]"#));
        assert_eq!(
            input.choices(),
            vec![
                (0, "Off".to_string()),
                (1, "1".to_string()),
                (2, "2".to_string()),
            ],
            "a short LABELS array must not truncate the selectable values"
        );
    }

    #[test]
    fn choices_treats_labels_alone_as_an_index_list() {
        let input = long_input(None, Some(r#"["First", "Second"]"#));
        assert_eq!(
            input.choices(),
            vec![(0, "First".to_string()), (1, "Second".to_string())]
        );
    }

    #[test]
    fn choices_is_empty_without_options_or_for_other_types() {
        assert!(
            long_input(None, None).choices().is_empty(),
            "a long with no options falls back to a stepper, not an empty combo"
        );
        let float: ISFInput =
            serde_json::from_str(r#"{"NAME": "speed", "TYPE": "float", "VALUES": [1, 2]}"#)
                .unwrap();
        assert!(
            float.choices().is_empty(),
            "only long inputs are enums, whatever else declares VALUES"
        );
    }

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
        assert!(
            pi.iter().all(|p| p.multiply_by.is_empty()),
            "MULTIPLY_BY is optional and absent here"
        );
    }

    #[test]
    fn parse_phase_input_multiply_by_accepts_string_or_array() {
        let json = r#"{
            "DESCRIPTION": "Test shader",
            "INPUTS": [],
            "PHASE_INPUTS": [
                {"PARAM": "speed", "MULTIPLY_BY": "rot_speed", "INDEX": 1, "SCALE": 0.2},
                {"PARAM": "speed", "MULTIPLY_BY": ["time_scale", "flow_speed"], "INDEX": 2}
            ]
        }"#;
        let meta: ISFMetadata = serde_json::from_str(json).unwrap();
        let pi = meta.phase_inputs.unwrap();
        assert_eq!(pi[0].param, "speed");
        assert_eq!(pi[0].multiply_by, vec!["rot_speed"]);
        assert_eq!(pi[0].index, 1);
        assert!((pi[0].scale - 0.2).abs() < 1e-5);
        assert_eq!(pi[1].multiply_by, vec!["time_scale", "flow_speed"]);
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
                    "OPTIONS": { "resolution": "half" },
                    "PARAM_BINDINGS": { "threshold": "edge_threshold" },
                    "PHASE_BINDINGS": { "orbit_phase": 2 }
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
        assert_eq!(
            meta.preprocessors[0].param_bindings.get("threshold"),
            Some(&"edge_threshold".to_string())
        );
        assert_eq!(
            meta.preprocessors[0].phase_bindings.get("orbit_phase"),
            Some(&2)
        );
        assert_eq!(meta.preprocessors[1].name, "edges");
        assert_eq!(meta.preprocessors[1].options, serde_json::Value::Null);
        assert!(meta.preprocessors[1].param_bindings.is_empty());
        assert!(meta.preprocessors[1].phase_bindings.is_empty());
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
