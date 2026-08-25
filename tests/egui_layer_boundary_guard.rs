//! Guards CONTRIBUTING.md: egui stays in `src/usecases/`. `src/internal/` and
//! `src/app/` must not name egui types in code. Doc comments may mention egui.

use std::path::Path;

const FORBIDDEN: [&str; 2] = ["egui::", "egui_wgpu::"];

fn is_code_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    !trimmed.is_empty()
        && !trimmed.starts_with("//")
        && !trimmed.starts_with("///")
        && !trimmed.starts_with("//!")
        && !trimmed.starts_with('*')
}

fn check_dir(dir: &Path, violations: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            check_dir(&path, violations);
            continue;
        }
        if path.extension().and_then(|n| n.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        for (lineno, line) in src.lines().enumerate() {
            if !is_code_line(line) {
                continue;
            }
            for needle in FORBIDDEN {
                if line.contains(needle) {
                    violations.push(format!("{}:{}: {line}", path.display(), lineno + 1));
                }
            }
        }
    }
}

#[test]
fn internal_and_app_do_not_use_egui_types() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    check_dir(&root.join("internal"), &mut violations);
    check_dir(&root.join("app"), &mut violations);
    assert!(
        violations.is_empty(),
        "src/internal and src/app must not name egui types \
         (CONTRIBUTING.md, /spec/app-presentation-boundary.md); found:\n{}",
        violations.join("\n")
    );
}
