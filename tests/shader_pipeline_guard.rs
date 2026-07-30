//! Guard: every shipped shader must build a pipeline *and* survive a frame.
//!
//! Building a pipeline only validates the bind group layout against the shader's
//! declarations. It cannot catch anything that goes wrong while encoding a
//! frame — most importantly resource-usage conflicts, which wgpu raises at
//! render time and which are configured to abort the process.
//!
//! The bug that motivated the render half: every pass buffer is bound as a
//! sampled texture on every pass, and non-persistent pass buffers used a single
//! texture for both reading and writing. Any shader declaring one therefore
//! bound it as COLOR_TARGET and RESOURCE simultaneously and killed the app on
//! its first frame. No shipped shader had a non-persistent pass, so nothing
//! caught it until one was written — and a build-only check never would have.
//!
//! Shaders are loaded from disk rather than the registry so a shader that fails
//! to parse is a failure here too, not a silent omission.

use varda::isf::ISFShader;
use varda::renderer::GpuContext;

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
    let Ok(gpu) = GpuContext::new_headless() else {
        eprintln!("no headless adapter; skipping");
        return;
    };
    let mut checked = 0;
    let mut failures = Vec::new();
    for path in shader_paths() {
        match ISFShader::from_file(path.to_str().unwrap()) {
            Ok(shader) => {
                // Transitions take two inputs and are built by the mixer, not as
                // decks or effects; mirror `ShaderRegistry`'s own generator/filter
                // split rather than inventing a third case.
                if shader.metadata.is_transition() {
                    continue;
                }
                checked += 1;
                let is_gen = shader.metadata.is_generator();
                let name = shader.name();
                let built = if is_gen {
                    varda::deck::Deck::new(&gpu, shader, W, H).map(|_| ())
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
    let Ok(gpu) = GpuContext::new_headless() else {
        eprintln!("no headless adapter; skipping");
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
        let Ok(mut deck) = varda::deck::Deck::new(&gpu, shader, W, H) else {
            continue; // build failures are the other test's business
        };

        // Two frames, not one: ping-pong pass buffers only alias on the second
        // frame if `swap()` is wrong, and a shader reading its own history
        // exercises a different path once history exists.
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
        // A GPU error no longer aborts the process — the deck is quarantined so
        // a live show survives it. That containment would otherwise make this
        // test blind, so assert on the quarantine rather than on not crashing.
        assert!(
            deck.gpu_error().is_none(),
            "{name}: quarantined by a GPU error during render: {}",
            deck.gpu_error().unwrap_or_default()
        );
        rendered += 1;
    }
    assert!(rendered > 30, "only rendered {rendered} generators");
}
