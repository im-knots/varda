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
        self.categories.as_ref().map_or(false, |cats| {
            cats.iter().any(|c| c.eq_ignore_ascii_case("transition"))
        })
    }

    /// Check if this shader is audio reactive
    pub fn is_audio_reactive(&self) -> bool {
        if let Some(inputs) = &self.inputs {
            inputs.iter().any(|input| {
                input.input_type == "audio" || input.input_type == "audioFFT"
            })
        } else {
            false
        }
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
        assert!((pi[1].scale - 1.0).abs() < 1e-5, "Default scale should be 1.0");
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