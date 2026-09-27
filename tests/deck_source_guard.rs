//! Guard: no code outside a deck source's own provider names its type.
//!
//! Adding a deck source means writing a provider next to its backend and one
//! line in `app/sources.rs`. That only stays true while the engine, the GUI and
//! the API treat every source through the provider traits, so a source type id
//! (the `type` tag a scene file saves) written anywhere else is a special case
//! creeping back in. See /spec/deck-source-providers.md § Guard scope.
//!
//! Test code is exempt: tests build decks of specific types on purpose. So are
//! the deprecated per-type API aliases, which exist to name types and go away
//! after one release.

use std::path::{Path, PathBuf};

/// Every source type id, as `app/sources.rs` registers them.
const SOURCE_TYPES: &[&str] = &[
    "Shader",
    "Image",
    "Video",
    "SolidColor",
    "Camera",
    "DepthSensor",
    "ScreenCapture",
    "Tap",
    "Ndi",
    "Syphon",
    "Spout",
    "Srt",
    "Hls",
    "Dash",
    "Rtmp",
    "Html",
];

/// Where an id may be written, relative to `src/`: each provider beside its
/// backend, and the registration list.
const ALLOWED: &[&str] = &[
    "app/sources.rs",
    "internal/generator",
    "internal/still",
    "internal/video",
    "internal/solid_color.rs",
    "internal/camera",
    "internal/depth",
    "internal/screen_capture",
    "internal/tap",
    "internal/ndi",
    "internal/syphon",
    "internal/spout",
    "internal/stream",
    "internal/html",
    // One-release aliases for the old per-type routes (Decision 6).
    "usecases/api/routes/deprecated_sources.rs",
    // Test fixtures: compiled only for tests and the `test-fixtures` feature.
    "usecases/ui/fixtures.rs",
];

/// A file that writes an id's spelling for something else, with why.
const SAME_SPELLING: &[(&str, &str, &str)] = &[
    (
        "usecases/ui/panels/outputs.rs",
        "Syphon",
        "the Syphon output protocol's label",
    ),
    (
        "usecases/ui/panels/outputs.rs",
        "Spout",
        "the Spout output protocol's label",
    ),
];

fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
    {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The part of a file that ships: everything before its first `#[cfg(test)]`.
/// Test modules sit at the end of a file by convention, and a file that is a
/// test module in its own right (`tests.rs`) is skipped entirely.
fn shipped_code(path: &Path, text: &str) -> Option<String> {
    if path.file_name().is_some_and(|n| n == "tests.rs") {
        return None;
    }
    let end = text.find("#[cfg(test)]").unwrap_or(text.len());
    Some(text[..end].to_string())
}

#[test]
fn source_type_ids_appear_only_in_their_providers() {
    let root = src_root();
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(
        !files.is_empty(),
        "no sources found under {}",
        root.display()
    );

    let mut offenders = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .expect("under src/")
            .to_string_lossy()
            .replace('\\', "/");
        if ALLOWED.iter().any(|allowed| rel.starts_with(allowed)) {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable source");
        let Some(code) = shipped_code(&path, &text) else {
            continue;
        };
        for (line_no, line) in code.lines().enumerate() {
            let code_part = line.split("//").next().unwrap_or_default();
            for id in SOURCE_TYPES {
                let same_spelling = SAME_SPELLING
                    .iter()
                    .any(|(file, word, _)| *file == rel && word == id);
                if !same_spelling && code_part.contains(&format!("\"{id}\"")) {
                    offenders.push(format!("src/{rel}:{}: \"{id}\"", line_no + 1));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "source type ids written outside their providers; route the behavior \
         through the provider traits instead:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_guard_lists_every_registered_type() {
    let sources =
        std::fs::read_to_string(src_root().join("app/sources.rs")).expect("app/sources.rs");
    let registered = sources
        .split("#[cfg(test)]")
        .nth(1)
        .expect("app/sources.rs lists its ids in a test");
    for id in SOURCE_TYPES {
        assert!(
            registered.contains(&format!("\"{id}\"")),
            "{id} is guarded here but not registered in app/sources.rs"
        );
    }
}
