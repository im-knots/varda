//! Scratch workspace allocation for tests.
//!
//! Available to unit tests via `cfg(test)` and to integration tests via the
//! `test-fixtures` feature, which `[dev-dependencies]` turns on for the
//! self-referential `varda` dependency.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// A fresh, isolated workspace root for one `VardaApp`.
///
/// Every test that constructs a `VardaApp` must pass this to `--workspace`.
/// Without the flag, [`crate::app::AppConfig`] resolves the workspace from the
/// current directory when it contains a `.varda/`, and under `cargo test` the
/// current directory is the crate root — so on a developer machine with a real
/// workspace checked out beside the source, the engine binds to their live show
/// data. One test that saves is then enough to overwrite scene, stage, MIDI,
/// keymap, and OSC config with engine defaults. That has happened; see
/// spec/persistence.md § Test isolation.
///
/// Each call returns its own directory, so tests stay independent of each other
/// and of execution order even when they save. The directories live under a
/// single per-process temporary root that is intentionally never cleaned up:
/// the callers hand back only the app, so there is no owner to tie a
/// [`tempfile::TempDir`] guard to.
///
/// # Panics
///
/// Panics if the temporary directory cannot be created, or if its path is not
/// valid UTF-8 — both of which make the calling test unrunnable anyway.
pub fn temp_workspace() -> String {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let root = ROOT.get_or_init(|| {
        tempfile::Builder::new()
            .prefix("varda-test-workspace-")
            .tempdir()
            .expect("create the per-process test workspace root")
    });

    let dir: PathBuf = root
        .path()
        .join(format!("ws{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).expect("create a test workspace");
    dir.into_os_string()
        .into_string()
        .expect("temporary directory path is valid UTF-8")
}

/// The standard configuration for a test-owned headless engine: no OSC, NDI, or
/// Syphon, and a scratch workspace of its own from [`temp_workspace`].
///
/// Prefer this over hand-rolling the flag list. Every argument is load-bearing —
/// omitting `--workspace` in particular is the mistake this helper exists to
/// make impossible.
///
/// # Panics
///
/// Panics if the scratch workspace cannot be created.
pub fn headless_config() -> crate::app::AppConfig {
    use clap::Parser;
    crate::app::AppConfig::parse_from([
        "varda",
        "--headless",
        "--no-osc",
        "--no-ndi",
        "--no-syphon",
        "--workspace",
        &temp_workspace(),
    ])
}

/// Run the reference-orbit kernel without analyzer-thread or texture-packing
/// overhead. Criterion uses this to isolate host arithmetic.
#[doc(hidden)]
pub fn benchmark_fractal_reference_orbit(iterations: usize) -> usize {
    crate::internal::analyzer::fractal_reference_orbit::benchmark_reference_orbit(iterations)
}

/// Run the deterministic 15-ray outward-rounded directional fixture set.
#[doc(hidden)]
pub fn benchmark_fractal_directional_certificates() -> usize {
    use crate::internal::analyzer::fractal_certification::segment::{
        certify_directional_ray_segment, DirectionalCertificateResult,
    };
    use crate::internal::analyzer::fractal_reference_orbit::StackParams;

    let params = StackParams {
        formulas: [5, 0, 0, 0],
        rates: [1, 0, 0, 0],
        power: 2.5,
        bailout: 0.1,
        max_iters: 1,
        refine: false,
        ..StackParams::default()
    };
    (0..15)
        .filter(|index| {
            let offset = f64::from(*index) * 0.002;
            let origin = [0.8 + offset, 0.6 - offset * 0.5, 0.4 + offset * 0.25];
            let direction = [0.2, -0.1 + offset * 0.1, 0.15];
            matches!(
                certify_directional_ray_segment(
                    &params,
                    &origin.map(|value| value.to_string()),
                    direction,
                    1,
                    1.0 / 64.0,
                    128,
                ),
                DirectionalCertificateResult::Certified(_)
            )
        })
        .count()
}

/// Readiness and host-side diagnostics for the fractal reference-orbit
/// preprocessor.
///
/// This deliberately exposes values rather than analyzer implementation types,
/// keeping benchmarks independent of the internal worker and snapshot APIs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FractalPreprocessorMetrics {
    pub payload_ready: bool,
    pub boundary_ready: bool,
    pub boundary_reason: u8,
    pub boundary_runtime_ms: f32,
    pub segment_ready: bool,
    pub segment_coverage: f32,
    /// True while the published orbit is the shallow provisional stand-in.
    pub payload_provisional: bool,
}

/// Return the latest fractal preprocessor metrics for `deck`.
///
/// `None` means the declared preprocessor has not been started (or its worker
/// has already exited).
#[doc(hidden)]
pub fn fractal_preprocessor_metrics(
    deck: &crate::deck::Deck,
) -> Option<FractalPreprocessorMetrics> {
    let snapshot = deck.analyzers.latest_snapshot("fractal_reference_orbit")?;
    Some(fractal_metrics_from_snapshot(&snapshot))
}

fn fractal_metrics_from_snapshot(
    snapshot: &crate::internal::analyzer::traits::AnalyzerSnapshot,
) -> FractalPreprocessorMetrics {
    FractalPreprocessorMetrics {
        payload_ready: snapshot.textures.contains_key("refOrbit"),
        boundary_ready: snapshot.scalar("boundary_ready") >= 1.0,
        boundary_reason: snapshot.scalar("boundary_reason").clamp(0.0, 255.0) as u8,
        boundary_runtime_ms: snapshot.scalar("boundary_runtime_ms"),
        segment_ready: snapshot.scalar("segment_ready") >= 1.0,
        segment_coverage: snapshot.scalar("segment_coverage"),
        payload_provisional: snapshot.scalar("payload_provisional") >= 0.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two apps built in the same process must not share a workspace, or a save
    /// in one becomes a load in the other.
    #[test]
    fn each_call_gets_its_own_directory() {
        let a = temp_workspace();
        let b = temp_workspace();
        assert_ne!(a, b);
        assert!(std::path::Path::new(&a).is_dir());
        assert!(std::path::Path::new(&b).is_dir());
    }

    /// The whole point: never the crate root, whose `.varda/` may be a real one.
    #[test]
    fn never_resolves_to_the_crate_root() {
        let ws = temp_workspace();
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(
            !std::path::Path::new(&ws).starts_with(crate_root),
            "test workspaces must live outside the source tree, got {ws}"
        );
    }

    /// The config must carry the scratch workspace all the way through
    /// resolution — an explicit `--workspace` outranks the CWD tier that would
    /// otherwise find the developer's `.varda/`.
    #[test]
    fn headless_config_resolves_to_a_scratch_workspace() {
        let root = headless_config().effective_workspace_root();
        assert!(
            !root.starts_with(env!("CARGO_MANIFEST_DIR")),
            "a test engine must never resolve the source tree as its workspace, got {}",
            root.display()
        );
        assert!(root.is_dir());
    }

    #[test]
    fn fractal_metrics_preserve_readiness_and_host_diagnostics() {
        use std::collections::HashMap;

        let snapshot = crate::internal::analyzer::traits::AnalyzerSnapshot {
            scalars: HashMap::from([
                ("boundary_ready".into(), 1.0),
                ("boundary_reason".into(), 7.0),
                ("boundary_runtime_ms".into(), 123.5),
                ("segment_ready".into(), 1.0),
                ("segment_coverage".into(), 0.625),
            ]),
            textures: HashMap::from([(
                "refOrbit".into(),
                crate::internal::analyzer::traits::TextureData {
                    generation: 1,
                    width: 1,
                    height: 1,
                    format: "rgba8unorm".into(),
                    data: vec![0; 4].into(),
                },
            )]),
            timestamp: std::time::Instant::now(),
        };

        assert_eq!(
            fractal_metrics_from_snapshot(&snapshot),
            FractalPreprocessorMetrics {
                payload_ready: true,
                boundary_ready: true,
                boundary_reason: 7,
                boundary_runtime_ms: 123.5,
                segment_ready: true,
                segment_coverage: 0.625,
                payload_provisional: false,
            }
        );
    }

    #[test]
    fn directional_benchmark_fixture_set_is_fully_certified() {
        assert_eq!(benchmark_fractal_directional_certificates(), 15);
    }
}
