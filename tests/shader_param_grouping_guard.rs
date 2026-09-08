//! Guard: the shipped shader library's `GROUP` conventions.
//!
//! See `/spec/parameter-inspector.md` § Library conventions. `GROUP` itself is
//! free text and stays that way, because rejecting an unknown group name would
//! break the community shaders Varda commits to loading. These rules apply to
//! the bundled library only, which is what a performer actually meets.
//!
//! The rule that is not stylistic is the last one. `hidden_prefixes` in
//! `src/usecases/ui/widgets.rs` hides `<prefix>_*` when a bool `<prefix>_mode`
//! is false. Put that gate in a different group from what it gates and the
//! performer gets a section that will not open, with the switch that opens it
//! filed somewhere else. Nothing crashes and no pixel test notices.
//!
//! Pure source analysis: no GPU, runs everywhere including CI.

use std::path::{Path, PathBuf};

/// At or above this many uniform parameters, a flat list stops being scannable
/// and the shader must declare groups. See the spec for why fourteen.
const GROUPING_REQUIRED_AT: usize = 14;

/// ISF types that never reach the params column: they are textures, not
/// uniform-block members, and `ShaderParams::from_inputs` skips them.
const NON_PARAM_TYPES: [&str; 3] = ["image", "audio", "audioFFT"];

fn shader_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read shaders/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|s| s.to_str()), Some("fs" | "comp")))
        .collect();
    files.sort();
    assert!(
        files.len() > 100,
        "expected the bundled library, found {}",
        files.len()
    );
    files
}

/// Extract the `/*{ ... }*/` ISF metadata header.
fn header(src: &str) -> Option<&str> {
    let start = src.find("/*{")?;
    let end = src[start..].find("}*/")? + start + 1;
    Some(&src[start + 2..end])
}

/// One `INPUTS` entry, reduced to what these rules care about.
struct Input {
    name: String,
    ty: String,
    group: Option<String>,
    default_false_bool: bool,
}

/// The inputs that reach the params column, in declaration order.
fn params(path: &Path) -> Vec<Input> {
    let src = std::fs::read_to_string(path).expect("read shader");
    let Some(hdr) = header(&src) else {
        return Vec::new();
    };
    let meta: serde_json::Value = serde_json::from_str(hdr)
        .unwrap_or_else(|e| panic!("{}: ISF header is not valid JSON: {e}", path.display()));
    let Some(inputs) = meta.get("INPUTS").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    inputs
        .iter()
        .filter_map(|i| {
            let ty = i.get("TYPE")?.as_str()?.to_string();
            if NON_PARAM_TYPES.contains(&ty.as_str()) {
                return None;
            }
            Some(Input {
                name: i.get("NAME")?.as_str()?.to_string(),
                default_false_bool: ty == "bool"
                    && i.get("DEFAULT").and_then(serde_json::Value::as_bool) == Some(false),
                ty,
                group: i
                    .get("GROUP")
                    .and_then(|g| g.as_str())
                    .map(ToString::to_string),
            })
        })
        .collect()
}

/// Named groups in first-appearance order, which is the order they render in.
fn group_order(params: &[Input]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for g in params.iter().filter_map(|p| p.group.as_deref()) {
        if !out.contains(&g) {
            out.push(g);
        }
    }
    out
}

fn shader_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into()
}

#[test]
fn large_shaders_declare_groups() {
    let mut offenders = Vec::new();
    for path in shader_files() {
        let params = params(&path);
        if params.len() >= GROUPING_REQUIRED_AT && group_order(&params).is_empty() {
            offenders.push(format!(
                "{} declares {} parameters and no GROUP",
                shader_name(&path),
                params.len()
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "a shader with {GROUPING_REQUIRED_AT} or more parameters must section them with GROUP \
         (see /spec/parameter-inspector.md § Library conventions):\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn grouped_shaders_keep_an_ungrouped_row() {
    let mut offenders = Vec::new();
    for path in shader_files() {
        let params = params(&path);
        if group_order(&params).is_empty() {
            continue;
        }
        if params.iter().all(|p| p.group.is_some()) {
            offenders.push(shader_name(&path));
        }
    }
    assert!(
        offenders.is_empty(),
        "ungrouped parameters render first, headerless and uncollapsible, so they are the row a \
         performer reaches for mid-set; these shaders left none:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn no_group_holds_a_single_parameter() {
    let mut offenders = Vec::new();
    for path in shader_files() {
        let params = params(&path);
        for group in group_order(&params) {
            let members = params
                .iter()
                .filter(|p| p.group.as_deref() == Some(group))
                .count();
            if members == 1 {
                offenders.push(format!("{} group '{group}'", shader_name(&path)));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a collapsing header over one row costs a click and saves no space; fold these into a \
         neighbouring group:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn mode_gate_shares_a_group_with_what_it_gates() {
    let mut offenders = Vec::new();
    for path in shader_files() {
        let params = params(&path);
        for gate in params.iter().filter(|p| p.ty == "bool") {
            let Some(stem) = gate.name.strip_suffix("_mode") else {
                continue;
            };
            let prefix = format!("{stem}_");
            for gated in params
                .iter()
                .filter(|p| p.name != gate.name && p.name.starts_with(&prefix))
            {
                if gated.group != gate.group {
                    offenders.push(format!(
                        "{}: gate '{}' is in {:?} but hides '{}' in {:?}",
                        shader_name(&path),
                        gate.name,
                        gate.group,
                        gated.name,
                        gated.group
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a `<prefix>_mode` bool hides `<prefix>_*`, so filing it away from what it hides leaves a \
         section that cannot be opened:\n  {}",
        offenders.join("\n  ")
    );
}

/// The gate rule only bites when a gate is actually off by default, since that
/// is when the section is hidden on load. Kept separate so the library cannot
/// quietly lose its only worked example of the interaction.
#[test]
fn chroma_flow_palette_gate_stays_with_its_colours() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/chroma_flow.fs");
    let params = params(&path);
    let gate = params
        .iter()
        .find(|p| p.name == "palette_mode")
        .expect("chroma_flow.fs declares palette_mode");
    assert!(
        gate.default_false_bool,
        "palette_mode is the library's worked example of a gate that is off by default"
    );
    for gated in params.iter().filter(|p| p.name.starts_with("palette_")) {
        assert_eq!(
            gated.group, gate.group,
            "'{}' is hidden by palette_mode and must share its group",
            gated.name
        );
    }
}
