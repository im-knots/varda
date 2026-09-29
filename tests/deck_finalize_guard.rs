//! Every shader, image, or video deck is finalized, and only the engine builds
//! them.
//!
//! `VardaApp::finalize_new_deck` starts CPU analyzers and acquires devices a
//! shader's `PREPROCESSORS` block requires; a deck that skips it renders blank
//! 1x1 textures with no error. The background loader (`app/deck_loads.rs`) is
//! the only construction path, so this checks it finalizes and reports
//! failures, and that no consumer builds decks itself.
//!
//! Targets are directories where possible, read recursively, so splitting a
//! module into submodules doesn't break the guard.

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
fn the_engine_loader_finalizes_every_deck_and_reports_failure() {
    let source = read("app/deck_loads.rs");
    let idx = source
        .find("finalize_new_deck")
        .expect("the engine loader finalizes decks before attaching them");
    let attach = &source[idx..];
    assert!(
        attach.contains("add_deck"),
        "finalize must run before the deck joins its channel"
    );
    assert!(
        source.contains("Failed to load deck"),
        "a failed attach must be surfaced to the operator"
    );
}

#[test]
fn consumers_do_not_build_decks() {
    let source = read("usecases");
    for needle in ["finalize_new_deck", "Deck::new", "Deck::new_from_"] {
        assert!(
            !source.contains(needle),
            "consumers send deck-creating commands; `{needle}` belongs to the engine loader"
        );
    }
}
