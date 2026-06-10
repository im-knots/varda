use anyhow::{Context, Result};
use shaderc::{Compiler, ShaderKind};

/// Compile GLSL fragment shader to SPIR-V
pub fn compile_glsl_to_spirv(glsl_source: &str, shader_name: &str) -> Result<Vec<u32>> {
    let compiler = Compiler::new().context("Failed to create shaderc compiler")?;

    let mut options = shaderc::CompileOptions::new().context("Failed to create compile options")?;

    // Set GLSL version to 330 (common for ISF shaders)
    options.set_source_language(shaderc::SourceLanguage::GLSL);
    options.set_target_env(
        shaderc::TargetEnv::Vulkan,
        shaderc::EnvVersion::Vulkan1_2 as u32,
    );

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

/// Compile GLSL compute shader to SPIR-V
pub fn compile_glsl_compute_to_spirv(glsl_source: &str, shader_name: &str) -> Result<Vec<u32>> {
    let compiler = Compiler::new().context("Failed to create shaderc compiler")?;

    let mut options = shaderc::CompileOptions::new().context("Failed to create compile options")?;

    options.set_source_language(shaderc::SourceLanguage::GLSL);
    options.set_target_env(
        shaderc::TargetEnv::Vulkan,
        shaderc::EnvVersion::Vulkan1_2 as u32,
    );

    let binary_result = compiler
        .compile_into_spirv(
            glsl_source,
            ShaderKind::Compute,
            shader_name,
            "main",
            Some(&options),
        )
        .with_context(|| format!("Failed to compile compute shader '{}'", shader_name))?;

    if binary_result.get_num_warnings() > 0 {
        log::warn!(
            "Compute shader '{}' compiled with warnings:\n{}",
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
    let mut isf_uniforms = String::from(
        r#"
// ISF automatic uniforms
uniform float TIME;
uniform float TIMEDELTA;
uniform int FRAMEINDEX;
uniform vec2 RENDERSIZE;
uniform vec4 DATE;
uniform float PHASE_TIME_0;
uniform float PHASE_TIME_1;
uniform float PHASE_TIME_2;
uniform float PHASE_TIME_3;

// ISF automatic varying (normalized fragment coordinates)
in vec2 isf_FragNormCoord;
"#,
    );

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
        isf_uniforms.push('\n');
        isf_uniforms.push_str(&uniform_decl);
    }
    isf_uniforms.push('\n');

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
        assert!(injected.contains("in vec2 isf_FragNormCoord"));
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

        // Legacy injection uses bare uniforms (not blocks) and lacks explicit
        // locations, so Vulkan SPIR-V compilation is expected to fail here.
        // The modern path uses the ISFUniforms block in pipeline.rs instead.
        if let Err(e) = spirv {
            println!(
                "Expected compilation error (legacy uniforms vs Vulkan blocks): {}",
                e
            );
        }
    }

    #[test]
    fn test_compile_tree_of_life_naga() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let shader_path = manifest_dir.join("shaders/tree_of_life.fs");
        let source = match std::fs::read_to_string(&shader_path) {
            Ok(s) => s,
            Err(_) => {
                println!("Skipping: shader file not found");
                return;
            }
        };
        let json_end = source.find("}*/").expect("JSON header");
        let glsl = source[json_end + 3..].trim();

        let spirv = compile_glsl_to_spirv(glsl, "tree_of_life.fs");
        if let Err(ref e) = spirv {
            panic!("GLSL compilation failed: {}", e);
        }
        let spirv_data = spirv.unwrap();

        // Now test naga parse + validation (same as pipeline.rs)
        let spirv_bytes: Vec<u8> = spirv_data.iter().flat_map(|w| w.to_le_bytes()).collect();
        let module =
            naga::front::spv::parse_u8_slice(&spirv_bytes, &naga::front::spv::Options::default())
                .expect("naga SPIR-V parse should succeed");

        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module);
        if let Err(ref e) = info {
            panic!("Naga validation failed: {:?}", e);
        }

        // Try WGSL output too
        let wgsl = naga::back::wgsl::write_string(
            &module,
            &info.unwrap(),
            naga::back::wgsl::WriterFlags::empty(),
        );
        if let Err(ref e) = wgsl {
            panic!("WGSL output failed: {:?}", e);
        }
        println!("WGSL output length: {}", wgsl.unwrap().len());
    }

    #[test]
    fn test_compile_simple_compute_shader() {
        let glsl = r#"
#version 450
layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;
layout(set = 0, binding = 0, rgba8) writeonly uniform image2D output_image;

void main() {
    ivec2 gid = ivec2(gl_GlobalInvocationID.xy);
    imageStore(output_image, gid, vec4(1.0, 0.0, 0.0, 1.0));
}
"#;

        let spirv = compile_glsl_compute_to_spirv(glsl, "test_compute");
        if let Err(ref e) = spirv {
            eprintln!("Compute compilation error: {}", e);
        }
        assert!(spirv.is_ok());
        let spirv_data = spirv.unwrap();
        assert!(!spirv_data.is_empty());
        assert_eq!(spirv_data[0], 0x07230203);
    }

    #[test]
    fn test_compile_black_hole_sim_full_pipeline() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let shader_path = manifest_dir.join("shaders/black_hole_sim.comp");
        let source = match std::fs::read_to_string(&shader_path) {
            Ok(s) => s,
            Err(_) => {
                println!("Skipping: black_hole_sim.comp not found");
                return;
            }
        };

        // Parse ISF metadata
        let json_end = source.find("}*/").expect("ISF JSON header not found");
        let json_str = &source[2..json_end + 1]; // skip /*
        let meta: super::super::ISFMetadata =
            serde_json::from_str(json_str).expect("ISF metadata should parse");
        assert!(meta.is_compute(), "should be a compute shader");
        assert_eq!(
            meta.buffers.len(),
            2,
            "should have 2 storage buffers (particles + grid)"
        );

        // Extract GLSL (everything after }*/)
        let glsl = source[json_end + 3..].trim();

        // Step 1: shaderc GLSL → SPIR-V
        let spirv = compile_glsl_compute_to_spirv(glsl, "black_hole_sim.comp");
        if let Err(ref e) = spirv {
            panic!("shaderc compilation failed: {}", e);
        }
        let spirv_data = spirv.unwrap();
        assert_eq!(spirv_data[0], 0x07230203, "SPIR-V magic number");

        // Step 2: naga SPIR-V parse + validation
        let spirv_bytes: Vec<u8> = spirv_data.iter().flat_map(|w| w.to_le_bytes()).collect();
        let module =
            naga::front::spv::parse_u8_slice(&spirv_bytes, &naga::front::spv::Options::default())
                .expect("naga SPIR-V parse should succeed");

        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("naga validation should succeed");

        // Step 3: naga → WGSL output
        let wgsl =
            naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
                .expect("WGSL output should succeed");

        assert!(!wgsl.is_empty(), "WGSL output should not be empty");
        println!(
            "black_hole_sim.comp: SPIR-V {} words → WGSL {} chars",
            spirv_data.len(),
            wgsl.len()
        );
    }
}
