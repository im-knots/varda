use super::metadata::ISFMetadata;
use anyhow::{Context, Result};
use std::path::Path;

/// Parsed ISF shader file
#[derive(Debug, Clone)]
pub struct ISFShader {
    /// Parsed metadata from JSON header
    pub metadata: ISFMetadata,
    
    /// GLSL fragment shader source code
    pub fragment_source: String,
    
    /// Optional vertex shader source code
    pub vertex_source: Option<String>,
    
    /// File path (for debugging/hot-reload)
    pub file_path: Option<String>,
}

impl ISFShader {
    /// Parse an ISF shader from a file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read ISF file: {}", path.display()))?;
        
        let mut shader = Self::from_string(&content)?;
        shader.file_path = Some(path.to_string_lossy().to_string());
        Ok(shader)
    }

    /// Parse an ISF shader from a string
    pub fn from_string(content: &str) -> Result<Self> {
        // ISF files have a JSON comment block at the top
        // Format: /*{ ... }*/
        let (metadata, fragment_source) = extract_json_and_glsl(content)?;
        
        Ok(ISFShader {
            metadata,
            fragment_source,
            vertex_source: None,
            file_path: None,
        })
    }

    /// Get shader name from metadata or filename
    pub fn name(&self) -> String {
        if let Some(path) = &self.file_path {
            Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unnamed")
                .to_string()
        } else {
            "Unnamed".to_string()
        }
    }

    /// Get shader description
    pub fn description(&self) -> String {
        self.metadata
            .description
            .clone()
            .unwrap_or_else(|| "No description".to_string())
    }

    /// Get shader author/credit
    pub fn credit(&self) -> String {
        self.metadata
            .credit
            .clone()
            .unwrap_or_else(|| "Unknown".to_string())
    }
}

/// Extract JSON metadata and GLSL code from ISF file content
fn extract_json_and_glsl(content: &str) -> Result<(ISFMetadata, String)> {
    // Find the JSON comment block: /*{ ... }*/
    let json_start = content.find("/*{")
        .context("ISF file must start with JSON comment block /*{ ... }*/")?;
    
    let json_end = content[json_start..].find("}*/")
        .context("ISF JSON comment block not properly closed with }*/")?;
    
    // Extract JSON (including the braces)
    let json_str = &content[json_start + 2..json_start + json_end + 1]; // Skip "/*" and include "}"
    
    // Parse JSON metadata
    let metadata: ISFMetadata = serde_json::from_str(json_str)
        .context("Failed to parse ISF JSON metadata")?;
    
    // Extract GLSL code (everything after the JSON block)
    let glsl_start = json_start + json_end + 3; // Skip "}*/"
    let fragment_source = content[glsl_start..].trim().to_string();
    
    Ok((metadata, fragment_source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_isf() {
        let isf_content = r#"/*{
    "DESCRIPTION": "Test shader",
    "CREDIT": "Test Author",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {
            "NAME": "color",
            "TYPE": "color",
            "DEFAULT": [1.0, 0.0, 0.0, 1.0]
        }
    ]
}*/

void main() {
    gl_FragColor = color;
}
"#;

        let shader = ISFShader::from_string(isf_content).unwrap();
        assert_eq!(shader.metadata.description, Some("Test shader".to_string()));
        assert_eq!(shader.metadata.credit, Some("Test Author".to_string()));
        assert!(shader.fragment_source.contains("void main()"));
    }

    #[test]
    fn test_is_generator() {
        let generator_isf = r#"/*{
    "CATEGORIES": ["Generator"],
    "INPUTS": [{"NAME": "time", "TYPE": "float"}]
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(generator_isf).unwrap();
        assert!(shader.metadata.is_generator());
    }

    #[test]
    fn test_is_filter() {
        let filter_isf = r#"/*{
    "CATEGORIES": ["Filter"],
    "INPUTS": [{"NAME": "inputImage", "TYPE": "image"}]
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(filter_isf).unwrap();
        assert!(shader.metadata.is_filter());
    }

    #[test]
    fn test_is_transition() {
        let isf = r#"/*{
    "CATEGORIES": ["Transition"],
    "INPUTS": [{"NAME": "inputImage", "TYPE": "image"}, {"NAME": "startImage", "TYPE": "image"}]
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(isf).unwrap();
        assert!(shader.metadata.is_transition());
        assert!(shader.metadata.is_filter()); // Transitions are also filters
    }

    #[test]
    fn test_is_not_transition() {
        let isf = r#"/*{
    "CATEGORIES": ["Generator"]
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(isf).unwrap();
        assert!(!shader.metadata.is_transition());
    }

    #[test]
    fn test_is_audio_reactive() {
        let isf = r#"/*{
    "INPUTS": [{"NAME": "audio", "TYPE": "audio"}, {"NAME": "fft", "TYPE": "audioFFT"}]
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(isf).unwrap();
        assert!(shader.metadata.is_audio_reactive());
    }

    #[test]
    fn test_is_not_audio_reactive() {
        let isf = r#"/*{
    "INPUTS": [{"NAME": "brightness", "TYPE": "float"}]
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(isf).unwrap();
        assert!(!shader.metadata.is_audio_reactive());
    }

    #[test]
    fn test_multiple_inputs() {
        let isf = r#"/*{
    "INPUTS": [
        {"NAME": "brightness", "TYPE": "float", "MIN": 0.0, "MAX": 1.0, "DEFAULT": 0.5},
        {"NAME": "invert", "TYPE": "bool", "DEFAULT": false},
        {"NAME": "mode", "TYPE": "long", "VALUES": [0, 1, 2], "LABELS": ["A", "B", "C"]},
        {"NAME": "tint", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0]},
        {"NAME": "center", "TYPE": "point2D", "DEFAULT": [0.5, 0.5]}
    ]
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(isf).unwrap();
        let inputs = shader.metadata.inputs.unwrap();
        assert_eq!(inputs.len(), 5);
        assert_eq!(inputs[0].name, "brightness");
        assert_eq!(inputs[0].min, Some(0.0));
        assert_eq!(inputs[0].max, Some(1.0));
        assert_eq!(inputs[2].values.as_ref().unwrap().len(), 3);
        assert_eq!(inputs[2].labels.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_no_inputs() {
        let isf = r#"/*{
    "DESCRIPTION": "Minimal"
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(isf).unwrap();
        assert!(shader.metadata.inputs.is_none());
        assert!(shader.metadata.is_generator()); // No image inputs = generator
        assert!(!shader.metadata.is_audio_reactive());
    }

    #[test]
    fn test_categories_string() {
        let isf = r#"/*{
    "CATEGORIES": ["Generator", "Color"]
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(isf).unwrap();
        assert_eq!(shader.metadata.categories_string(), "Generator, Color");
    }

    #[test]
    fn test_categories_string_empty() {
        let isf = r#"/*{
    "DESCRIPTION": "No categories"
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(isf).unwrap();
        assert_eq!(shader.metadata.categories_string(), "Uncategorized");
    }

    #[test]
    fn test_shader_name_from_path() {
        let mut shader = ISFShader::from_string("/*{}*/\nvoid main() {}").unwrap();
        shader.file_path = Some("/path/to/Color Bars.fs".into());
        assert_eq!(shader.name(), "Color Bars");
    }

    #[test]
    fn test_shader_name_no_path() {
        let shader = ISFShader::from_string("/*{}*/\nvoid main() {}").unwrap();
        assert_eq!(shader.name(), "Unnamed");
    }

    #[test]
    fn test_description_and_credit() {
        let isf = r#"/*{
    "DESCRIPTION": "A cool effect",
    "CREDIT": "Test Author"
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(isf).unwrap();
        assert_eq!(shader.description(), "A cool effect");
        assert_eq!(shader.credit(), "Test Author");
    }

    #[test]
    fn test_description_and_credit_defaults() {
        let shader = ISFShader::from_string("/*{}*/\nvoid main() {}").unwrap();
        assert_eq!(shader.description(), "No description");
        assert_eq!(shader.credit(), "Unknown");
    }

    #[test]
    fn test_missing_json_block() {
        let result = ISFShader::from_string("void main() {}");
        assert!(result.is_err());
    }

    #[test]
    fn test_unclosed_json_block() {
        let result = ISFShader::from_string("/*{\nvoid main() {}");
        assert!(result.is_err());
    }

    #[test]
    fn test_fragment_source_trimmed() {
        let isf = "/*{}*/\n\n  void main() { }\n\n";
        let shader = ISFShader::from_string(isf).unwrap();
        assert_eq!(shader.fragment_source, "void main() { }");
    }

    #[test]
    fn test_passes_metadata() {
        let isf = r#"/*{
    "PASSES": [
        {"TARGET": "buffer1", "PERSISTENT": true},
        {}
    ]
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(isf).unwrap();
        let passes = shader.metadata.passes.unwrap();
        assert_eq!(passes.len(), 2);
        assert_eq!(passes[0].target, Some("buffer1".into()));
        assert_eq!(passes[0].persistent, Some(true));
        assert!(passes[1].target.is_none());
    }

    #[test]
    fn test_transition_case_insensitive() {
        let isf = r#"/*{
    "CATEGORIES": ["transition"]
}*/
void main() {}
"#;
        let shader = ISFShader::from_string(isf).unwrap();
        assert!(shader.metadata.is_transition());
    }
}
