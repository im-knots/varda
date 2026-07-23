//! Guards the dependency inversion fixed by /spec/engine-value-types.md: the
//! engine contract layer (`src/engine/{mod,types,traits}.rs`) must name its
//! value vocabulary from `engine::value` (a leaf with no dependency on
//! `internal`), never reach back into the `renderer`/`surface`/`video`
//! implementation modules. `engine::value`'s own files are exempt — they are
//! the destination, and their doc comments may discuss the modules they
//! replaced. This is an integration test (not `#[cfg(test)]` inside
//! `src/engine`) so the forbidden-path string literals it greps for don't
//! trip the guard against itself.

use std::path::Path;

const FORBIDDEN: [&str; 3] = ["crate::renderer::", "crate::surface::", "crate::video::"];

fn check_dir(dir: &Path, violations: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read_dir engine/") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("value") {
                continue; // engine::value is the destination, not a violator.
            }
            check_dir(&path, violations);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read engine source file");
        for (lineno, line) in src.lines().enumerate() {
            for needle in FORBIDDEN {
                if line.contains(needle) {
                    violations.push(format!("{}:{}: `{}`", path.display(), lineno + 1, line));
                }
            }
        }
    }
}

#[test]
fn engine_names_no_internal_infra_modules() {
    let engine_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/engine");
    let mut violations = Vec::new();
    check_dir(&engine_dir, &mut violations);
    assert!(
        violations.is_empty(),
        "src/engine/** must not name crate::renderer::/crate::surface::/crate::video:: \
         (dependency inversion per /spec/engine-value-types.md); found:\n{}",
        violations.join("\n")
    );
}
