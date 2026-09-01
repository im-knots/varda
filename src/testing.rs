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
}
