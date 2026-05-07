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

