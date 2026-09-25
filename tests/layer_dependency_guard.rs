//! Guards the dependency rule in /spec/clean-architecture.md (Decision #11):
//! engine-owned code (`src/engine`, `src/internal`, `src/app`) never names a
//! `usecases` type. Consumers depend on the engine, never the reverse. Doc
//! comments may mention the consumers.

use std::path::Path;

const FORBIDDEN: &str = "usecases::";

fn is_code_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    !trimmed.is_empty() && !trimmed.starts_with("//") && !trimmed.starts_with('*')
}

fn check_dir(dir: &Path, violations: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            check_dir(&path, violations);
            continue;
        }
        if path.extension().and_then(|n| n.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        for (lineno, line) in src.lines().enumerate() {
            if is_code_line(line) && line.contains(FORBIDDEN) {
                violations.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    lineno + 1,
                    line.trim()
                ));
            }
        }
    }
}

#[test]
fn engine_owned_code_does_not_name_consumer_types() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    for layer in ["engine", "internal", "app"] {
        check_dir(&root.join(layer), &mut violations);
    }
    assert!(
        violations.is_empty(),
        "engine-owned code must not name usecases types \
         (/spec/clean-architecture.md Decision #11); found:\n{}",
        violations.join("\n")
    );
}

/// Consumers change engine state with commands, never through `&mut` access to
/// a subsystem (/spec/clean-architecture.md Decision #12). The accessors are
/// `pub(crate)`, which the compiler still lets `usecases` call, so this guards
/// the rest.
#[test]
fn consumers_do_not_take_mutable_engine_access() {
    const MUTATORS: [&str; 5] = [
        "mixer_mut(",
        "camera_manager_mut(",
        "screen_capture_manager_mut(",
        "depth_manager_mut(",
        ".open_camera(",
    ];
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    collect_calls(&root.join("usecases"), &MUTATORS, &mut violations);
    assert!(
        violations.is_empty(),
        "consumers must send commands instead of mutating engine subsystems \
         (/spec/clean-architecture.md Decision #12); found:\n{}",
        violations.join("\n")
    );
}

fn collect_calls(dir: &Path, needles: &[&str], violations: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_calls(&path, needles, violations);
            continue;
        }
        if path.extension().and_then(|n| n.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        for (lineno, line) in src.lines().enumerate() {
            if is_code_line(line) && needles.iter().any(|n| line.contains(n)) {
                violations.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    lineno + 1,
                    line.trim()
                ));
            }
        }
    }
}
