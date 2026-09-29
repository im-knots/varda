//! Every shipped shader must build a pipeline and survive a frame.
//!
//! Building a pipeline only checks the bind group layout. Errors raised while
//! encoding a frame, such as resource-usage conflicts, need a render. For
//! example, a pass buffer used as both `COLOR_TARGET` and sampled texture in one
//! pass fails only at render time.
//!
//! Shaders load from disk rather than the registry, so a parse failure fails
//! here too.

use varda::isf::ISFShader;

mod common;
use common::headless_gpu;

/// Small enough to keep 130+ shaders fast, large enough that fixed-size passes
/// and derivative-based effects still have somewhere to render.
const W: u32 = 64;
const H: u32 = 64;

fn shader_paths() -> Vec<std::path::PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("shaders dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("fs"))
        .collect();
    paths.sort();
    paths
}

#[test]
fn every_shipped_shader_builds_a_pipeline() {
    let Some(gpu) = headless_gpu() else {
        return;
    };
    let mut checked = 0;
    let mut failures = Vec::new();
    for path in shader_paths() {
        match ISFShader::from_file(path.to_str().unwrap()) {
            Ok(shader) => {
                // Transitions take two inputs and are built by the mixer; this
                // mirrors `ShaderRegistry`'s generator/filter split.
                if shader.metadata.is_transition() {
                    continue;
                }
                checked += 1;
                let is_gen = shader.metadata.is_generator();
                let name = shader.name();
                let built = if is_gen {
                    varda::deck::Deck::from_shader(&gpu, shader, W, H).map(|_| ())
                } else {
                    varda::deck::Effect::new(&gpu, shader).map(|_| ())
                };
                if let Err(e) = built {
                    failures.push(format!("{name}: {e:#}"));
                }
            }
            Err(e) => failures.push(format!("{}: parse: {e:#}", path.display())),
        }
    }
    assert!(checked > 100, "only found {checked} shaders");
    assert!(
        failures.is_empty(),
        "shaders failed to build:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_generator_survives_a_rendered_frame() {
    let Some(gpu) = headless_gpu() else {
        return;
    };
    let audio = varda::audio::AudioData::default();
    let modulation = varda::modulation::ModulationEngine::new();

    let mut rendered = 0;
    for path in shader_paths() {
        let Ok(shader) = ISFShader::from_file(path.to_str().unwrap()) else {
            continue;
        };
        if !shader.metadata.is_generator() || shader.metadata.is_compute() {
            continue;
        }
        let name = shader.name();
        let Ok(mut deck) = varda::deck::Deck::from_shader(&gpu, shader, W, H) else {
            continue; // build failures are covered by the other test
        };

        // Two frames: ping-pong pass buffers alias only on the second frame if
        // `swap()` is wrong, and history reads take a different path once
        // history exists.
        let mut cmds = Vec::new();
        for _ in 0..2 {
            deck.render(&gpu, &audio, &modulation, 0, &mut cmds)
                .unwrap_or_else(|e| panic!("{name}: render failed: {e:#}"));
        }
        gpu.queue.submit(cmds);
        let _ = gpu.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(5)),
        });
        // GPU errors quarantine the deck instead of aborting, so check the
        // quarantine rather than a crash.
        assert!(
            deck.gpu_error().is_none(),
            "{name}: quarantined by a GPU error during render: {}",
            deck.gpu_error().unwrap_or_default()
        );
        rendered += 1;
    }
    assert!(rendered > 30, "only rendered {rendered} generators");
}
