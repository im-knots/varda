//! Guard: every path that constructs a `Deck` must finalize it.
//!
//! Post-construction wiring — starting CPU analyzers and acquiring devices a
//! shader's `PREPROCESSORS` block requires — lives in
//! `VardaApp::finalize_new_deck`. There are two construction paths and they
//! silently diverged once already: the synchronous `add_deck` command called the
//! wiring, and the UI's background loader (`spawn_deck_loads`, completed in
//! `usecases/ui/runner.rs`) did not. The symptom was a `depth_sensor` shader
//! dragged from the Library rendering against blank 1x1 textures with no error
//! toast and nothing in the log — it just looked like a shader with a flat
//! background.
//!
//! Nothing about that divergence was type-visible, so this is a source guard:
//! it fails if a third path adds a deck to a channel without finalizing first.

use std::path::{Path, PathBuf};

fn src(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(rel)
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(src(rel)).unwrap_or_else(|e| panic!("reading {rel}: {e}"))
}

#[test]
fn both_deck_construction_paths_call_finalize_new_deck() {
    for (file, context) in [
        ("app/engine_impl.rs", "the synchronous AddDeck command"),
        ("usecases/ui/runner.rs", "the UI background loader"),
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
    let source = read("usecases/ui/runner.rs");
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
