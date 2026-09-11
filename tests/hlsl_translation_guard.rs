//! Guard: every shipped shader must translate to HLSL.
//!
//! Varda compiles ISF GLSL to SPIR-V with shaderc, and wgpu then lowers that to
//! the backend's own language: MSL on Metal, HLSL on DX12. naga's HLSL backend
//! does not implement everything its SPIR-V frontend accepts, so a shader can
//! compile and run perfectly on macOS and Linux and produce an **invalid
//! pipeline** on Windows, where the only symptom is a validation error at
//! `set_pipeline` naming no shader at all.
//!
//! Varda's tests only started running on Windows recently, so this class had
//! never been checked. Adding this found `warped_grid.fs` calling `inverse()`,
//! for which naga reports `Unimplemented("write_expr_math Inverse")`. It had
//! been shipping broken on DX12.
//!
//! The point of running the translation here is that it needs no Windows and no
//! GPU: it is the same lowering DX12 performs, run on whatever machine is to
//! hand. It does **not** cover what happens after, when FXC or DXC compiles the
//! HLSL, which has its own limits. A pass here means the shader is expressible
//! in HLSL, not that every DX12 driver will accept it.

use std::path::Path;

#[test]
fn every_shipped_shader_translates_to_hlsl() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("shaders dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("fs"))
        .collect();
    paths.sort();
    assert!(paths.len() > 100, "only found {} shaders", paths.len());

    let mut failures = Vec::new();
    for path in &paths {
        let name = path
            .file_name()
            .expect("shader file name")
            .to_string_lossy()
            .to_string();
        let shader = varda::isf::ISFShader::from_file(path)
            .unwrap_or_else(|e| panic!("{name}: parse failed: {e:#}"));
        let spirv = varda::isf::compile_glsl_to_spirv(&shader.fragment_source, &shader.name())
            .unwrap_or_else(|e| panic!("{name}: glsl to spirv failed: {e:#}"));

        let module = naga::front::spv::parse_u8_slice(
            bytemuck::cast_slice(&spirv),
            &naga::front::spv::Options::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: spirv to naga failed: {e:?}"));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: naga validation failed: {e:?}"));

        let mut hlsl = String::new();
        let options = naga::back::hlsl::Options::default();
        let pipeline_options = naga::back::hlsl::PipelineOptions::default();
        let mut writer = naga::back::hlsl::Writer::new(&mut hlsl, &options, &pipeline_options);
        if let Err(e) = writer.write(&module, &info, None) {
            failures.push(format!("  {name}: {e:?}"));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} shaders cannot be translated to HLSL, so their pipelines are \
         invalid on DX12:\n{}",
        failures.len(),
        paths.len(),
        failures.join("\n")
    );
}
