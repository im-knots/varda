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
}

