//! Queue submits and surface configures go through `SubmitGate`. One that
//! bypasses it brings back an intermittent "Failed to wait for GPU to come
//! idle" error that no test can trigger, since tests have no window.

use std::path::Path;

const GATED_CALLS: [&str; 2] = ["queue.submit(", "surface.configure("];

/// Files allowed to call these directly, each with the reason.
const ALLOWED: [(&str, &str); 3] = [
    ("internal/renderer/submit_gate.rs", "the gate itself"),
    (
        "internal/renderer/context.rs",
        "configures the first surface before any other thread exists",
    ),
    ("testing.rs", "single-threaded bench and test helpers"),
];

fn is_code_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    !trimmed.is_empty() && !trimmed.starts_with("//") && !trimmed.starts_with('*')
}

fn check_dir(root: &Path, dir: &Path, violations: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            check_dir(root, &path, violations);
            continue;
        }
        if path.extension().and_then(|n| n.to_str()) != Some("rs") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        if ALLOWED.iter().any(|(allowed, _)| *allowed == relative) {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        for (lineno, line) in src.lines().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            if is_code_line(line) && GATED_CALLS.iter().any(|call| line.contains(call)) {
                violations.push(format!("{relative}:{}: {}", lineno + 1, line.trim()));
            }
        }
    }
}

#[test]
fn submits_and_surface_configures_go_through_the_gate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    check_dir(&root, &root, &mut violations);
    assert!(
        violations.is_empty(),
        "call SubmitGate::submit and SubmitGate::configure instead; found:\n{}",
        violations.join("\n")
    );
}
