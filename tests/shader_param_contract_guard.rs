//! Guard: every shader must declare its ISF params with the GLSL types the
//! engine actually writes. See `docs/12-isf-authoring.md` § Input Types.
//!
//! This is a *value* contract, not a validation one — a mismatch compiles and
//! links cleanly, builds a valid pipeline, and then silently misbehaves at
//! runtime. It cannot be caught by pipeline creation or by pixel tests that
//! don't happen to exercise the affected parameter.
//!
//! The bug this was written for: `invert.fs` (and four others) declared ISF
//! `bool` inputs as `float`, but `ParamValue::Bool` is written as a `u32`. The
//! bytes `01 00 00 00` reinterpreted as an IEEE-754 float are `1.4e-45` — a
//! denormal that fails `> 0.5` — so every such toggle was permanently stuck
//! off and `invert.fs` was a no-op.
//!
//! Pure source analysis: no GPU, runs everywhere including CI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// ISF type → the GLSL scalar/vector type the engine's byte encoding requires.
/// Mirrors the table in `docs/12-isf-authoring.md` and `ParamValue::write_bytes`.
fn required_glsl_type(isf_type: &str) -> Option<&'static str> {
    match isf_type {
        "float" => Some("float"),
        "bool" => Some("uint"),
        "long" => Some("int"),
        "color" => Some("vec4"),
        "point2D" => Some("vec2"),
        _ => None, // image/audio/audioFFT are textures, not uniform-block members
    }
}

fn shader_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Extract the `/*{ ... }*/` ISF metadata header.
fn header(src: &str) -> Option<&str> {
    let start = src.find("/*{")?;
    let end = src[start..].find("}*/")? + start + 1;
    Some(&src[start + 2..end])
}

/// Collect `name -> declared_glsl_type` from every `uniform <Block> { ... }`.
/// The params block is variously named `UserParams`, `TransitionParams`, etc.,
/// so scan all uniform blocks rather than hard-coding one name.
fn declared_types(src: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = src;
    while let Some(u) = rest.find("uniform ") {
        rest = &rest[u + 8..];
        let Some(open) = rest.find('{') else { break };
        // Reject `uniform sampler foo;` / `uniform texture2D foo;` (no block).
        if rest[..open].contains(';') {
            continue;
        }
        let Some(close) = rest.find('}') else { break };
        for line in rest[open + 1..close].lines() {
            let line = line.split("//").next().unwrap_or("").trim();
            let line = line.strip_suffix(';').unwrap_or(line);
            let mut it = line.split_whitespace();
            if let (Some(ty), Some(name)) = (it.next(), it.next()) {
                if it.next().is_none() {
                    out.insert(name.to_string(), ty.to_string());
                }
            }
        }
        rest = &rest[close..];
    }
    out
}

#[test]
fn isf_params_are_declared_with_the_glsl_types_the_engine_writes() {
    let dir = shader_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
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

    let mut violations: Vec<String> = Vec::new();
    let mut checked = 0usize;
    let mut bools_checked = 0usize;

    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(path).expect("read shader");
        let Some(hdr) = header(&src) else {
            violations.push(format!("{name}: no ISF metadata header"));
            continue;
        };
        let meta: serde_json::Value = match serde_json::from_str(hdr) {
            Ok(v) => v,
            Err(e) => {
                violations.push(format!("{name}: bad JSON header: {e}"));
                continue;
            }
        };
        let Some(inputs) = meta.get("INPUTS").and_then(|i| i.as_array()) else {
            continue;
        };
        let declared = declared_types(&src);

        for input in inputs {
            let (Some(pname), Some(ptype)) = (
                input.get("NAME").and_then(|v| v.as_str()),
                input.get("TYPE").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let Some(want) = required_glsl_type(ptype) else {
                continue;
            };
            checked += 1;
            if ptype == "bool" {
                bools_checked += 1;
            }
            match declared.get(pname) {
                // Unreferenced params are legal — the shader simply ignores them.
                None => {}
                Some(got) if got == want => {}
                Some(got) => violations.push(format!(
                    "{name}: input `{pname}` is ISF `{ptype}` (engine writes {want}) \
                     but the shader declares `{got}` — the bytes will be reinterpreted"
                )),
            }
        }
    }

    assert!(
        violations.is_empty(),
        "{} shader parameter type mismatch(es):\n  {}",
        violations.len(),
        violations.join("\n  ")
    );
    // Guard the guard: if parsing silently stops finding params this must fail.
    assert!(
        checked > 400,
        "expected to check hundreds of params, only saw {checked}"
    );
    assert!(
        bools_checked >= 10,
        "expected at least 10 bool params (the case that regressed), saw {bools_checked}"
    );
}
