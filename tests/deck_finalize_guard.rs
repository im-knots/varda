//! Guard: every path that constructs a `Deck` must finalize it.
//!
//! Post-construction wiring — starting CPU analyzers and acquiring devices a
//! shader's `PREPROCESSORS` block requires — lives in
//! `VardaApp::finalize_new_deck`. There are two construction paths and they
//! silently diverged once already: the synchronous `add_deck` command called the
//! wiring, and the UI's background loader (`spawn_deck_loads`, completed in
//! `usecases/ui/runner/`) did not. The symptom was a `depth_sensor` shader
//! dragged from the Library rendering against blank 1x1 textures with no error
//! toast and nothing in the log — it just looked like a shader with a flat
//! background.
//!
//! Nothing about that divergence was type-visible, so this is a source guard:
//! it fails if a third path adds a deck to a channel without finalizing first.
//!
//! Targets are named as directories where possible, and read recursively, so the
//! guard follows the code when it is split across submodules rather than failing
//! on a stale path.

use std::path::{Path, PathBuf};

fn src(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(rel)
}

/// Read one `.rs` file, or concatenate every `.rs` file beneath a directory.
fn read(rel: &str) -> String {
    let path = src(rel);
    if path.is_file() {
        return std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {rel}: {e}"));
    }
    let mut out = String::new();
    let mut stack = vec![path];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("reading dir {rel}: {e}"));
        for entry in entries {
            let child = entry.expect("dir entry").path();
            if child.is_dir() {
                stack.push(child);
            } else if child.extension().is_some_and(|ext| ext == "rs") {
                out.push_str(
                    &std::fs::read_to_string(&child)
                        .unwrap_or_else(|e| panic!("reading {}: {e}", child.display())),
                );
                out.push('\n');
            }
        }
    }
    assert!(!out.is_empty(), "no .rs files found under {rel}");
    out
}

#[test]
fn both_deck_construction_paths_call_finalize_new_deck() {
    for (file, context) in [
        ("app/engine_impl.rs", "the synchronous AddDeck command"),
        ("usecases/ui/runner", "the UI background loader"),
    ] {
        let source = read(file);
        assert!(
            source.contains("finalize_new_deck"),
            "{file} ({context}) no longer calls finalize_new_deck — a deck built \
             there will skip analyzer startup and required-device acquisition"
        );
    }
}

#[test]
fn background_loader_handles_the_finalize_error() {
    // Finalization fails when a required preprocessor cannot be satisfied. The
    // background loader must discard the deck and tell the performer rather than
    // adding a deck that cannot render.
    let source = read("usecases/ui/runner");
    let idx = source
        .find("finalize_new_deck")
        .expect("background loader finalizes");
    let after = &source[idx..(idx + 400).min(source.len())];
    assert!(
        after.contains("notify_error"),
        "the background loader must surface a finalize failure to the performer"
    );
    assert!(
        after.contains("continue"),
        "the background loader must discard a deck that failed to finalize"
    );
}
