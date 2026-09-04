//! Guard: every shipped shader's `UserParams` block must match the byte layout the
//! engine writes into it.
//!
//! The user-parameter binding is created with `min_binding_size: None`, so wgpu never
//! checks it. The engine serialises parameters in `INPUTS` order using std140 rules and
//! the shader reads them positionally, which means a shader that declares its block in a
//! different order — or that loses a member to SPIR-V dead-code elimination — reads
//! neighbouring parameters' bytes. Nothing crashes; the shader just silently misbehaves,
//! and `shader_pipeline_guard.rs` still passes because the pipeline builds and renders
//! fine.
//!
//! This became a live risk when parameters started moving into `PHASE_INPUTS`: the engine
//! integrates those itself, so the shader body stops referencing them and they become
//! exactly the dead uniforms the compiler is entitled to strip. See
//! [/spec/phase-accumulators.md](/spec/phase-accumulators.md).

use varda::isf::{ISFShader, compile_glsl_compute_to_spirv, compile_glsl_to_spirv};
use varda::params::ShaderParams;

fn shader_paths() -> Vec<std::path::PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("shaders dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("fs" | "comp")))
        .collect();
    paths.sort();
    paths
}

/// std140 offsets the engine will write, derived the same way `ShaderParams` does.
fn expected_offsets(params: &ShaderParams) -> Vec<(String, u32)> {
    let mut offset = 0u32;
    let mut out = Vec::new();
    for name in &params.param_order {
        let Some(value) = params.values.get(name) else {
            continue;
        };
        let (align, size) = match value {
            varda::params::ParamValue::Float(_)
            | varda::params::ParamValue::Bool(_)
            | varda::params::ParamValue::Long(_) => (4, 4),
            varda::params::ParamValue::Point2D(_) => (8, 8),
            varda::params::ParamValue::Color(_) => (16, 16),
        };
        offset = offset.div_ceil(align) * align;
        out.push((name.clone(), offset));
        offset += size;
    }
    out
}

/// Reflect the user-parameter uniform block out of compiled SPIR-V.
///
/// The engine binds this block by index — it is always the last uniform binding — not
/// by name, and shaders variously call it `UserParams`, `FilterParams`, or
/// `TransitionParams`. Match the engine: take the highest-bound uniform struct that
/// isn't `ISFUniforms`.
///
/// Returns `None` when the shader declares no such block, which is the correct state
/// for a shader with no parameters.
fn reflect_user_params(spirv: &[u32]) -> Option<Vec<(String, u32)>> {
    let bytes: Vec<u8> = spirv.iter().flat_map(|w| w.to_le_bytes()).collect();
    let module =
        naga::front::spv::parse_u8_slice(&bytes, &naga::front::spv::Options::default()).ok()?;

    let mut best: Option<(u32, Vec<(String, u32)>)> = None;
    for (_, var) in module.global_variables.iter() {
        if var.space != naga::AddressSpace::Uniform {
            continue;
        }
        let ty = &module.types[var.ty];
        let naga::TypeInner::Struct { members, .. } = &ty.inner else {
            continue;
        };
        if ty.name.as_deref() == Some("ISFUniforms") {
            continue;
        }
        let binding = var.binding.as_ref().map_or(0, |b| b.binding);
        let layout = members
            .iter()
            .map(|m| (m.name.clone().unwrap_or_default(), m.offset))
            .collect();
        if best.as_ref().is_none_or(|(b, _)| binding > *b) {
            best = Some((binding, layout));
        }
    }
    best.map(|(_, layout)| layout)
}

#[test]
fn every_shader_user_params_block_matches_engine_layout() {
    let mut checked = 0;
    let mut skipped_no_params = 0;
    let mut failures = Vec::new();

    for path in shader_paths() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let Ok(shader) = ISFShader::from_file(path.to_str().unwrap()) else {
            failures.push(format!("{name}: failed to parse"));
            continue;
        };

        let inputs = shader.metadata.inputs.as_deref().unwrap_or(&[]);
        let params = ShaderParams::from_inputs(inputs);
        if params.is_empty() {
            skipped_no_params += 1;
            continue;
        }

        let compiled = if shader.metadata.is_compute() {
            compile_glsl_compute_to_spirv(&shader.fragment_source, &name)
        } else {
            compile_glsl_to_spirv(&shader.fragment_source, &name)
        };
        let spirv = match compiled {
            Ok(spirv) => spirv,
            Err(err) => {
                failures.push(format!("{name}: failed to compile: {err:#}"));
                continue;
            }
        };

        let Some(actual) = reflect_user_params(&spirv) else {
            failures.push(format!(
                "{name}: declares {} parameter(s) but has no UserParams block",
                params.param_order.len()
            ));
            continue;
        };

        checked += 1;
        let expected = expected_offsets(&params);
        if actual.len() != expected.len() {
            failures.push(format!(
                "{name}: metadata declares {} parameter(s) but the shader block has {} member(s).\n  metadata: {:?}\n  shader:   {:?}",
                expected.len(),
                actual.len(),
                expected.iter().map(|(n, _)| n).collect::<Vec<_>>(),
                actual.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            ));
            continue;
        }
        for ((exp_name, exp_off), (act_name, act_off)) in expected.iter().zip(actual.iter()) {
            if exp_name != act_name || exp_off != act_off {
                failures.push(format!(
                    "{name}: parameter {exp_name} is written at offset {exp_off} but the shader reads {act_name} at offset {act_off}"
                ));
            }
        }
    }

    assert!(checked > 100, "only checked {checked} shaders with params");
    assert!(
        failures.is_empty(),
        "{} shader(s) disagree with the engine's parameter layout ({skipped_no_params} had no params):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
