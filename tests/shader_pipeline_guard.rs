//! Guard: every shipped shader must compile into a real GPU pipeline.
//!
//! The registry loads shaders lazily at runtime, so a shader with a bad binding
//! layout or a GLSL error ships silently and only fails when a performer drags
//! it onto a deck mid-set.

use varda::isf::ISFShader;
use varda::renderer::GpuContext;

#[test]
fn every_shipped_shader_builds_a_pipeline() {
    let Ok(gpu) = GpuContext::new_headless() else {
        eprintln!("no headless adapter; skipping");
        return;
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders");
    let mut checked = 0;
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("shaders dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("fs") {
            continue;
        }
        match ISFShader::from_file(path.to_str().unwrap()) {
            Ok(shader) => {
                // Transitions take two inputs and are built by the mixer, not
                // as deck sources or effects; mirror `ShaderRegistry`'s own
                // generator/filter split rather than inventing a third case.
                if shader.metadata.is_transition() {
                    continue;
                }
                checked += 1;
                let is_gen = shader.metadata.is_generator();
                let name = shader.name();
                let built = if is_gen {
                    varda::deck::Deck::new(&gpu, shader, 64, 64).map(|_| ())
                } else {
                    varda::deck::Effect::new(&gpu, shader).map(|_| ())
                };
                if let Err(e) = built {
                    failures.push(format!("{name}: {e}"));
                }
            }
            Err(e) => failures.push(format!("{}: parse: {e}", path.display())),
        }
    }
    assert!(checked > 100, "only found {checked} shaders");
    assert!(
        failures.is_empty(),
        "shaders failed to build:\n{}",
        failures.join("\n")
    );
}
