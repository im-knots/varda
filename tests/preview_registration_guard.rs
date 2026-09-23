//! Guards preview-texture ownership: `src/usecases/ui/runner/preview.rs` is the
//! only place that registers or re-points egui preview textures. Previews show
//! gamma-encoded `PreviewEncoder` targets, and a registration made anywhere else
//! against a raw (linear) render view is never replaced, because the sync pass
//! skips slots it already knows about.

use std::path::Path;

const REGISTRATION_CALLS: [&str; 2] = [
    "register_native_texture(",
    "update_egui_texture_from_wgpu_texture(",
];

/// Files allowed to register egui textures, each with the reason.
const ALLOWED: [(&str, &str); 2] = [
    (
        "usecases/ui/runner/preview.rs",
        "owns every preview registration",
    ),
    (
        "usecases/ui/runner/camera_detect.rs",
        "camera-detection overlay, not a preview slot",
    ),
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
            if is_code_line(line) && REGISTRATION_CALLS.iter().any(|call| line.contains(call)) {
                violations.push(format!("{relative}:{}: {}", lineno + 1, line.trim()));
            }
        }
    }
}

#[test]
fn only_the_preview_module_registers_preview_textures() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    check_dir(&root, &root, &mut violations);
    assert!(
        violations.is_empty(),
        "egui preview textures must be registered by \
         usecases/ui/runner/preview.rs (sync_preview_registrations); found:\n{}",
        violations.join("\n")
    );
}
