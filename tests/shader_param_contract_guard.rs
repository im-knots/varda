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

/// Every shader file, `.fs` and `.comp`.
fn shader_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(shader_dir())
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

/// The executable source: no ISF header, no uniform block declarations, no
/// comments. What is left is what actually runs, so a parameter appearing here
/// is a parameter the shader applies.
fn executable_body(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    // Drop `/*...*/` runs, which covers the ISF header and block comments alike.
    while let Some(open) = rest.find("/*") {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find("*/") else {
            rest = "";
            break;
        };
        rest = &rest[open + close + 2..];
    }
    out.push_str(rest);

    let no_line_comments: String = out
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");

    // Drop `uniform <Block> { ... }` declarations, keeping bare `uniform
    // sampler x;` lines (no block) as-is — they declare no parameters.
    let mut body = String::with_capacity(no_line_comments.len());
    let mut rest = no_line_comments.as_str();
    while let Some(u) = rest.find("uniform ") {
        let after = &rest[u + 8..];
        let Some(open) = after.find('{') else { break };
        if after[..open].contains(';') {
            body.push_str(&rest[..u + 8]);
            rest = after;
            continue;
        }
        let Some(close) = after.find('}') else { break };
        body.push_str(&rest[..u]);
        rest = &after[close + 1..];
    }
    body.push_str(rest);
    body
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
    let files = shader_files();
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

/// Guard: no shader claims a parameter name the playback modulation targets
/// have reserved. See `spec/video-playback-modulation.md` § Key naming.
///
/// Video playback keys live in the same `deck_<uuid>:<name>` namespace as a
/// deck's ISF generator inputs, so a shader input called `video_speed` would
/// resolve to the *same* modulation key as the clip's playback speed. One
/// assignment would then drive both, and neither the UI nor the engine has any
/// way to tell them apart: the map is keyed by string.
///
/// This cannot be caught downstream. The shader compiles, the pipeline builds,
/// and the cross-wiring only shows when someone assigns a modulator to one of
/// the two parameters on a deck that happens to have both.
///
/// Pure source analysis: no GPU, runs everywhere including CI.
#[test]
fn no_shader_input_claims_a_reserved_playback_modulation_name() {
    // Taken from the engine rather than retyped, so renaming a target cannot
    // leave this guard checking a name nothing uses any more.
    use varda::video::modulation as vm;
    let exact = [vm::SCALING_MODE];
    let prefix = "video_";

    let mut violations: Vec<String> = Vec::new();
    let mut inputs_checked = 0usize;

    for path in shader_files() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(&path).expect("read shader");
        let Some(hdr) = header(&src) else { continue };
        let Ok(meta) = serde_json::from_str::<serde_json::Value>(hdr) else {
            continue;
        };
        let Some(inputs) = meta.get("INPUTS").and_then(|i| i.as_array()) else {
            continue;
        };
        for input in inputs {
            let Some(pname) = input.get("NAME").and_then(|v| v.as_str()) else {
                continue;
            };
            inputs_checked += 1;
            if pname.starts_with(prefix) || exact.contains(&pname) {
                violations.push(format!(
                    "{name}: input `{pname}` collides with a reserved playback \
                     modulation target on the same deck key namespace — rename it"
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "{} reserved-name collision(s):\n  {}",
        violations.len(),
        violations.join("\n  ")
    );
    // Guard the guard: a parsing change that stops finding inputs must fail
    // here rather than pass vacuously.
    assert!(
        inputs_checked > 400,
        "expected to check hundreds of inputs, only saw {inputs_checked}"
    );
}

/// The reserved names must actually be distinct from each other, or two targets
/// would share a key and silently drive one another.
#[test]
fn reserved_playback_modulation_names_are_unique() {
    use varda::video::modulation as vm;
    let names = [
        vm::SPEED,
        vm::POSITION,
        vm::PLAY,
        vm::LOOP_MODE,
        vm::SCALING_MODE,
    ];
    let unique: std::collections::HashSet<&&str> = names.iter().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "reserved names collide: {names:?}"
    );
}

/// Is `word` a whole identifier at `at`, rather than part of a longer one?
fn is_word_boundary(src: &str, at: usize, len: usize) -> bool {
    let ident_char = |c: char| c.is_alphanumeric() || c == '_';
    let before = src[..at].chars().next_back().is_none_or(|c| !ident_char(c));
    let after = src[at + len..]
        .chars()
        .next()
        .is_none_or(|c| !ident_char(c));
    before && after
}

/// `expr` with the arguments of every function call removed, so that a value
/// consumed by `sin`, `mod` or `fract` no longer counts as present. Those bound
/// their result, and a bounded value is not a growing one.
fn without_call_arguments(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len());
    let mut chars = expr.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '(' {
            out.push(c);
            continue;
        }
        let callee = expr[..i]
            .trim_end()
            .ends_with(|c: char| c.is_alphanumeric() || c == '_');
        let mut depth = 1usize;
        let mut inner = String::new();
        for (_, c) in chars.by_ref() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                break;
            }
            inner.push(c);
        }
        // A bare grouping paren keeps its contents; a call swallows them.
        if callee {
            out.push_str(" () ");
        } else {
            out.push('(');
            out.push_str(&without_call_arguments(&inner));
            out.push(')');
        }
    }
    out
}

/// Top-level function bodies, so a local in one is not read as the same name in
/// another. `particle_collider.fs` has both an arc parameter `t` and, in
/// `main`, a phase alias `t`; without this split the first looks like the
/// second and the guard reports a bug in correct geometry code.
fn function_bodies(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (i, c) in body.char_indices() {
        match c {
            '{' => {
                if depth == 0 {
                    start = i + 1;
                }
                depth += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    out.push(&body[start..i]);
                }
            }
            _ => {}
        }
    }
    out
}

/// Names inside one function that hold an unbounded, growing time value: the
/// four accumulators plus anything assigned from one *outside* a call.
///
/// `float t = PHASE_TIME_0;` and `coord += PHASE_TIME_0;` both alias the phase.
/// `float f = fract(PHASE_TIME_0);` does not — `fract` bounds it.
fn growing_names(func: &str) -> std::collections::HashSet<String> {
    let mut names: std::collections::HashSet<String> =
        (0..4).map(|i| format!("PHASE_TIME_{i}")).collect();
    // Assignments can chain (`a = PHASE_TIME_0; b = a; c = b;`), so iterate.
    // Four passes is far past the depth any shipped shader uses.
    for _ in 0..4 {
        for stmt in func.split(';') {
            let Some((lhs, rhs)) = stmt.split_once('=') else {
                continue;
            };
            let lhs = lhs.trim_end();
            // Comparisons and `==`, not assignments.
            if lhs.ends_with(['!', '<', '>', '=']) || rhs.starts_with('=') {
                continue;
            }
            let Some(target) = lhs
                .trim_end_matches(['+', '-', '*', '/'])
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .rfind(|s| !s.is_empty())
            else {
                continue;
            };
            if names.contains(target) {
                continue;
            }
            let visible = without_call_arguments(rhs);
            if visible
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|tok| names.contains(tok))
            {
                names.insert(target.to_string());
            }
        }
    }
    names
}

/// Parameters in the multiplicative chain around the token at `[at, at+len)`.
///
/// Walks right through `* x`, `/ x` and `* (…)`, and left through `x *`, so
/// `PHASE_TIME_0 * 0.5 * look_speed` is caught even though the adjacent operand
/// is a constant. The walk stops at `+`, `-`, `,` or a closing paren, which is
/// what keeps the legitimate bounded case out: in `sin(PHASE_TIME_0) * animate`
/// the phase is immediately followed by `)`, so `animate` is never reached.
fn chain_params(expr: &str, at: usize, len: usize, is_param: &dyn Fn(&str) -> bool) -> Vec<String> {
    let bytes = expr.as_bytes();
    let ident_at = |s: &str| -> Vec<String> {
        s.split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|t| is_param(t))
            .map(str::to_string)
            .collect()
    };
    let mut found = Vec::new();

    let mut i = at + len;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || (bytes[i] != b'*' && bytes[i] != b'/') {
            break;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'(' {
            let mut depth = 0usize;
            let start = i;
            while i < bytes.len() {
                match bytes[i] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                i += 1;
                if depth == 0 {
                    break;
                }
            }
            found.extend(ident_at(&expr[start..i]));
        } else {
            // `.` is part of the operand so that a decimal literal or a
            // swizzle does not end the walk: `PHASE_TIME_0 * 0.5 * look_speed`
            // is exactly the shape that got past the adjacency-only check.
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            if i == start {
                break; // Not an operand we understand; stop rather than guess.
            }
            found.extend(ident_at(&expr[start..i]));
        }
    }

    // Leftward, multiplication only: `look_speed * PHASE_TIME_0`.
    let mut j = at;
    loop {
        while j > 0 && bytes[j - 1].is_ascii_whitespace() {
            j -= 1;
        }
        if j == 0 || bytes[j - 1] != b'*' {
            break;
        }
        j -= 1;
        while j > 0 && bytes[j - 1].is_ascii_whitespace() {
            j -= 1;
        }
        let end = j;
        while j > 0
            && (bytes[j - 1].is_ascii_alphanumeric()
                || bytes[j - 1] == b'_'
                || bytes[j - 1] == b'.')
        {
            j -= 1;
        }
        if j == end {
            break;
        }
        found.extend(ident_at(&expr[j..end]));
    }
    found
}

/// Every place a shader scales a growing time value by a live float parameter.
///
/// `PHASE_TIME_n` grows without bound, so multiplying it by a parameter means
/// changing that parameter multiplies a large number by a new factor and the
/// animation teleports — the further into a set you are, the bigger the jump.
/// That is the precise discontinuity the accumulator exists to remove, and
/// `MULTIPLY_BY` exists so the factor goes *inside* the integral instead.
///
/// Three things have to line up for this to be both sound and useful, and each
/// of them was a bug that shipped:
///
/// * The chain is walked to its end, not just the adjacent operand.
///   `steel_lattice.fs` wrote `rot(PHASE_TIME_0 * 0.5 * look_speed)`, where the
///   operand next to the phase is a harmless constant.
/// * Local aliases count. `liquid_light.fs` hid behind `float t =
///   PHASE_TIME_0;`, and `bars.fs`, `scanlines.fs` and `lines.fs` behind
///   `coord += PHASE_TIME_0;` followed by `fract(coord * bar_count)`.
/// * Aliases are per function, because names repeat. `particle_collider.fs`
///   has an arc parameter named `t` in one function and a phase alias named `t`
///   in another; conflating them reports a bug in correct code.
///
/// The remaining gap is interprocedural: `eyes.fs` passes `PHASE_TIME_0` into
/// `eye(…, float t)` and scales it by `blink_speed` in the callee, which is
/// invisible here. Following arguments across call sites is more machinery than
/// the bug rate justifies; the behavioural test in `tests/render_correctness.rs`
/// covers the class from the other side by measuring the jump.
fn phase_scaled_by_parameter(name: &str, src: &str) -> Vec<String> {
    let body = executable_body(src);
    let declared = declared_types(src);
    let is_param = |ident: &str| declared.get(ident).is_some_and(|t| t == "float");
    let mut found = Vec::new();

    for func in function_bodies(&body) {
        let growing = growing_names(func);
        for token in &growing {
            for (at, _) in func.match_indices(token.as_str()) {
                if !is_word_boundary(func, at, token.len()) {
                    continue;
                }
                for param in chain_params(func, at, token.len(), &is_param) {
                    found.push(format!(
                        "{name}: `{token} … {param}` scales a growing time value by a live \
                         parameter — put {param} inside the integral with MULTIPLY_BY, or \
                         split the integral if the rate is affine in it"
                    ));
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Guard: no shipped shader multiplies accumulated phase by a live parameter.
/// See `spec/phase-accumulators.md` § Authoring Rules.
///
/// Not catchable downstream: it compiles, links, builds a valid pipeline and
/// renders a plausible frame. Only moving the fader reveals it, which in
/// practice means only a performance does. `lagrangian.fs` and
/// `liquid_light.fs` both shipped with a version of it.
///
/// Pure source analysis: no GPU, runs everywhere including CI.
#[test]
fn no_shader_scales_accumulated_phase_by_a_parameter() {
    let mut violations: Vec<String> = Vec::new();
    let mut shaders_with_phase = 0usize;

    for path in shader_files() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(&path).expect("read shader");
        if !src.contains("PHASE_INPUTS") {
            continue;
        }
        shaders_with_phase += 1;
        violations.extend(phase_scaled_by_parameter(&name, &src));
    }

    assert!(
        violations.is_empty(),
        "{} phase accumulator violation(s):\n  {}",
        violations.len(),
        violations.join("\n  ")
    );
    assert!(
        shaders_with_phase >= 20,
        "expected the phased shaders to be found, saw {shaders_with_phase}"
    );
}

/// Wrap a `main` body in the smallest shader the detector will parse.
///
/// One uniform member per line: `declared_types` reads the block line by line,
/// as every shipped shader writes it.
fn probe(body: &str) -> String {
    format!(
        r#"/*{{
    "INPUTS": [
        {{"NAME": "rot_speed", "TYPE": "float", "DEFAULT": 1.0}},
        {{"NAME": "bar_count", "TYPE": "float", "DEFAULT": 8.0}}
    ],
    "PHASE_INPUTS": [{{"PARAM": "speed", "INDEX": 0}}]
}}*/
layout(set = 0, binding = 1) uniform UserParams {{
    float rot_speed;
    float bar_count;
}};
{body}"#
    )
}

/// The guard above passes vacuously — that is the point of it — so the detector
/// has to be shown to fail on the bugs it is meant to catch. Every positive
/// case here is distilled from a shader that actually shipped with it, and the
/// negative cases are shapes the library legitimately uses and must not lose.
#[test]
fn the_phase_scaling_detector_catches_the_bug_it_guards() {
    let must_flag: &[(&str, &str)] = &[
        (
            "the plain multiply",
            "void main() { float a = PHASE_TIME_0 * rot_speed; }",
        ),
        (
            // liquid_light.fs
            "a parenthesised affine factor",
            "void main() { float a = PHASE_TIME_0 * (1.0 + rot_speed * 0.8); }",
        ),
        (
            // steel_lattice.fs — the adjacent operand is an innocent constant,
            // so an adjacency-only check reads this as clean.
            "a parameter further down the chain",
            "void main() { float a = PHASE_TIME_0 * 0.5 * rot_speed; }",
        ),
        (
            // star_nest.fs
            "a local alias of the phase",
            "void main() { float t = PHASE_TIME_0; float a = t * rot_speed; }",
        ),
        (
            // bars.fs, scanlines.fs — the phase is folded into a coordinate and
            // the multiply happens later, inside a bounding call.
            "a coordinate the phase was added into",
            "void main() { float c = 0.5; c += PHASE_TIME_0; float a = fract(c * bar_count); }",
        ),
        (
            "the parameter on the left",
            "void main() { float a = rot_speed * PHASE_TIME_0; }",
        ),
        (
            "division, which teleports just as well",
            "void main() { float a = PHASE_TIME_0 / rot_speed; }",
        ),
    ];
    for (what, body) in must_flag {
        let hits = phase_scaled_by_parameter("bad.fs", &probe(body));
        assert!(!hits.is_empty(), "missed {what}: {body}");
    }

    let must_not_flag: &[(&str, &str)] = &[
        (
            // twist.fs — stepping an amplitude is a smoothing concern, not an
            // accumulator one. This is the case that rules out simply looking
            // for a parameter anywhere in the statement.
            "a bounded function of phase scaled by a parameter",
            "void main() { float a = sin(PHASE_TIME_0) * rot_speed; }",
        ),
        (
            "phase scaled by constants only",
            "void main() { float a = PHASE_TIME_0 * 0.11 + PHASE_TIME_0 * 2.0; }",
        ),
        (
            "an alias that a bounding call already tamed",
            "void main() { float f = fract(PHASE_TIME_0); float a = f * rot_speed; }",
        ),
        (
            // particle_collider.fs: `t` is an arc parameter in one function and
            // a phase alias in another. Without per-function scoping the arc
            // maths is reported as a teleport.
            "a same-named local in a different function",
            "float arcPt(float t) { return t * rot_speed; } \
             void main() { float t = PHASE_TIME_0; float a = sin(t); }",
        ),
        (
            // The fix for bars.fs: the count multiplies position, and the phase
            // is added after, with the count inside the integral.
            "the shape of the fix",
            "void main() { float c = 0.5; float a = fract(c * bar_count + PHASE_TIME_0); }",
        ),
    ];
    for (what, body) in must_not_flag {
        let hits = phase_scaled_by_parameter("good.fs", &probe(body));
        assert!(hits.is_empty(), "false positive on {what}: {hits:?}");
    }
}
