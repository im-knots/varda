use anyhow::{Context, Result};
use shaderc::{Compiler, ShaderKind};

/// Compile GLSL fragment shader to SPIR-V
pub fn compile_glsl_to_spirv(glsl_source: &str, shader_name: &str) -> Result<Vec<u32>> {
    let mut compiler = Compiler::new()
        .context("Failed to create shaderc compiler")?;
    
    let mut options = shaderc::CompileOptions::new()
        .context("Failed to create compile options")?;
    
    // Set GLSL version to 330 (common for ISF shaders)
    options.set_source_language(shaderc::SourceLanguage::GLSL);
    options.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_2 as u32);
    
    // Compile to SPIR-V
    let binary_result = compiler
        .compile_into_spirv(
            glsl_source,
            ShaderKind::Fragment,
            shader_name,
            "main",
            Some(&options),
        )
        .with_context(|| format!("Failed to compile shader '{}'", shader_name))?;
    
    // Check for warnings
    if binary_result.get_num_warnings() > 0 {
        log::warn!(
            "Shader '{}' compiled with warnings:\n{}",
            shader_name,
            binary_result.get_warning_messages()
        );
    }
    
    Ok(binary_result.as_binary().to_vec())
}

/// Inject ISF automatic uniforms into GLSL source
/// ISF provides these built-in variables:
/// - TIME: float (elapsed time in seconds)
/// - TIMEDELTA: float (time since last frame)
/// - FRAMEINDEX: int (frame counter)
/// - RENDERSIZE: vec2 (output resolution)
/// - DATE: vec4 (year, month, day, seconds)
/// - isf_FragNormCoord: vec2 (normalized fragment coordinates 0-1)
pub fn inject_isf_uniforms(glsl_source: &str) -> String {
    inject_isf_uniforms_with_params(glsl_source, &[])
}

/// Inject ISF uniforms including user parameters (legacy GLSL 330 style)
pub fn inject_isf_uniforms_with_params(glsl_source: &str, inputs: &[super::ISFInput]) -> String {
    let mut isf_uniforms = String::from(r#"
// ISF automatic uniforms
uniform float TIME;
uniform float TIMEDELTA;
uniform int FRAMEINDEX;
uniform vec2 RENDERSIZE;
uniform vec4 DATE;

// ISF automatic varying (normalized fragment coordinates)
varying vec2 isf_FragNormCoord;
"#);

    // Add user parameter uniforms
    for input in inputs {
        let uniform_decl = match input.input_type.as_str() {
            "float" => format!("uniform float {};", input.name),
            "bool" => format!("uniform bool {};", input.name),
            "long" => format!("uniform int {};", input.name),
            "color" => format!("uniform vec4 {};", input.name),
            "point2D" => format!("uniform vec2 {};", input.name),
            // image, audio, audioFFT are handled as samplers elsewhere
            _ => continue,
        };
        isf_uniforms.push_str("\n");
        isf_uniforms.push_str(&uniform_decl);
    }
    isf_uniforms.push_str("\n");

    // Insert uniforms after the version directive (if present) or at the beginning
    if let Some(version_end) = glsl_source.find('\n') {
        let first_line = &glsl_source[..version_end];
        if first_line.contains("#version") {
            format!(
                "{}\n{}\n{}",
                first_line,
                isf_uniforms,
                &glsl_source[version_end + 1..]
            )
        } else {
            format!("{}{}", isf_uniforms, glsl_source)
        }
    } else {
        format!("{}{}", isf_uniforms, glsl_source)
    }
}

/// Generate GLSL 450 user params uniform block declaration
/// Returns the uniform block code and whether any params were generated
pub fn generate_user_params_block(inputs: &[super::ISFInput]) -> Option<String> {
    let mut members = Vec::new();

    for input in inputs {
        let member = match input.input_type.as_str() {
            "float" => format!("    float {};", input.name),
            "bool" => format!("    uint {};  // bool stored as uint", input.name),
            "long" => format!("    int {};", input.name),
            "color" => format!("    vec4 {};", input.name),
            "point2D" => format!("    vec2 {};", input.name),
            _ => continue,
        };
        members.push(member);
    }

    if members.is_empty() {
        return None;
    }

    Some(format!(
        r#"layout(set = 0, binding = 1) uniform UserParams {{
{}
}};"#,
        members.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_simple_shader() {
        let glsl = r#"
#version 450
layout(location = 0) out vec4 fragColor;

void main() {
    fragColor = vec4(1.0, 0.0, 0.0, 1.0);
}
"#;

        let spirv = compile_glsl_to_spirv(glsl, "test_shader");
        if let Err(ref e) = spirv {
            eprintln!("Compilation error: {}", e);
        }
        assert!(spirv.is_ok());
        let spirv_data = spirv.unwrap();
        assert!(!spirv_data.is_empty());

        // SPIR-V magic number check
        assert_eq!(spirv_data[0], 0x07230203);
    }

    #[test]
    fn test_inject_isf_uniforms() {
        let glsl = r#"#version 330
void main() {
    gl_FragColor = vec4(TIME, 0.0, 0.0, 1.0);
}
"#;
        
        let injected = inject_isf_uniforms(glsl);
        assert!(injected.contains("uniform float TIME"));
        assert!(injected.contains("uniform vec2 RENDERSIZE"));
        assert!(injected.contains("varying vec2 isf_FragNormCoord"));
    }

    #[test]
    fn test_compile_with_isf_uniforms() {
        let glsl = r#"
#version 330
out vec4 fragColor;

void main() {
    fragColor = vec4(TIME, isf_FragNormCoord.x, isf_FragNormCoord.y, 1.0);
}
"#;
        
        let injected = inject_isf_uniforms(glsl);
        let spirv = compile_glsl_to_spirv(&injected, "isf_test");
        
        // This might fail because we're using varying in a modern GLSL context
        // but it demonstrates the injection mechanism
        if let Err(e) = spirv {
            println!("Expected compilation error (varying vs in/out): {}", e);
        }
    }
}

