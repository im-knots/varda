//! Writes address entities by UUID, never by position. Checked by scanning
//! source text:
//!
//!   1. `EngineCommand` has no positional identity field. Ordinals within an
//!      already-identified entity are allowed only if listed in
//!      `ALLOWED_ORDINALS`.
//!   2. No HTTP route path takes a numeric identity segment, and no handler is
//!      a `*_by_idx` variant.
//!
//! This lives in `tests/` so the forbidden literals it greps for don't match
//! this file.

use std::path::{Path, PathBuf};

/// Ordinals that give a position within an entity the command already
/// identifies by UUID. Listed explicitly so adding one shows in the diff.
///
/// - `from_idx` / `to_idx` — the two positions a reorder swaps.
/// - `step_idx` / `step_index` — a step's position within its sequence, as the
///   sequencer addresses steps (`GoTo { step_index }`).
/// - `corner_idx` — which corner of a four-corner warp.
/// - `after_vert_idx`, `edge_idx`, `anchor_idx`, `segment_idx`, `hole_index` —
///   coordinates within a surface's geometry, on a surface named by UUID.
/// - `target_idx` — a target's position in a macro's target list, on a macro
///   named by UUID. Macro targets have no identity of their own.
const ALLOWED_ORDINALS: [&str; 11] = [
    "from_idx",
    "to_idx",
    "step_idx",
    "step_index",
    "corner_idx",
    "after_vert_idx",
    "edge_idx",
    "anchor_idx",
    "segment_idx",
    "hole_index",
    "target_idx",
];

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    out
}

/// The field name in `name: Type,`, if the line looks like a struct-variant field.
fn field_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.starts_with("//") {
        return None;
    }
    let (name, _) = trimmed.split_once(':')?;
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        return None;
    }
    Some(name)
}

#[test]
fn engine_commands_carry_no_positional_identity() {
    let src = std::fs::read_to_string(manifest_path("src/engine/mod.rs")).expect("read engine/mod");
    // Scan only the command enum; the result and outcome types above it are
    // read-side.
    let start = src
        .find("pub enum EngineCommand")
        .expect("EngineCommand enum");
    let commands = &src[start..];

    let violations: Vec<String> = commands
        .lines()
        .enumerate()
        .filter_map(|(lineno, line)| {
            let name = field_name(line)?;
            let positional = name.ends_with("_idx") || name.ends_with("_index") || name == "idx";
            if !positional || ALLOWED_ORDINALS.contains(&name) {
                return None;
            }
            Some(format!(
                "src/engine/mod.rs:{}: field `{}`",
                start_line(&src, start) + lineno,
                name
            ))
        })
        .collect();

    assert!(
        violations.is_empty(),
        "EngineCommand must identify entities by UUID, not position. A new \
         positional field is either an identity (use a UUID instead) or an \
         ordinal (add it to ALLOWED_ORDINALS with a reason). Found:\n{}",
        violations.join("\n")
    );
}

/// 1-based line number of `offset` within `src`.
fn start_line(src: &str, offset: usize) -> usize {
    src[..offset].lines().count()
}

#[test]
fn http_routes_take_no_numeric_identity_segment() {
    // A path parameter named as a positional index, e.g.
    // "/api/channels/{channel_idx}/decks/{deck_idx}". `{step_idx}` is allowed;
    // see ALLOWED_ORDINALS.
    let banned_segments = [
        "{idx}",
        "{channel_idx}",
        "{deck_idx}",
        "{effect_idx}",
        "{output_idx}",
        "{sequence_idx}",
        "{surface_idx}",
        "{preset_idx}",
    ];
    let mut violations = Vec::new();

    for path in rust_files(&manifest_path("src/usecases/api")) {
        // Route tests name legacy paths in their assertions.
        if path.file_name().and_then(|n| n.to_str()) == Some("tests.rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read api source file");
        for (lineno, line) in src.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for banned in banned_segments {
                if line.contains(banned) {
                    violations.push(format!(
                        "{}:{}: route segment `{}`",
                        path.display(),
                        lineno + 1,
                        banned
                    ));
                }
            }
            if line.contains("by_idx") || line.contains("ByIdx") {
                violations.push(format!(
                    "{}:{}: index-addressed handler",
                    path.display(),
                    lineno + 1
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "HTTP writes address entities by UUID, not position; found:\n{}",
        violations.join("\n")
    );
}
