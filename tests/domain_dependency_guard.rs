//! Guards the module order in /spec/domain-dependencies.md: production code in
//! a domain module under `src/internal/` names only modules earlier in
//! [`ORDER`], and `engine::value` names none. Test code is exempt.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Every domain module, bottom to top, grouped by tier.
const ORDER: &[&str] = &[
    // 1. Foundations
    "ids",
    "files",
    "isf",
    "audio",
    "camera",
    "ndi",
    "stream",
    "html",
    "screen_capture",
    "recording",
    "notifications",
    "sysmon",
    "cli_install",
    "registry",
    // 2. Time
    "clock",
    "timebase",
    "transport",
    "timecode",
    // 3. GPU
    "surface",
    "renderer",
    "spout",
    "syphon",
    // 4. Signals
    "modulation",
    "params",
    "video",
    "analyzer",
    "depth",
    // 5. Composition
    "arrangement",
    "deck",
    "channel",
    "macros",
    "mixer",
    // 6. Control
    "param_router",
    "midi",
    "osc",
    "keymap",
    // 7. Saved files
    "scene",
    "persistence",
];

// ── Tokens ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Ident(String),
    PathSep,
    Punct(char),
}

#[derive(Debug, Clone)]
struct Tok {
    kind: Kind,
    line: usize,
}

impl Tok {
    fn is_ident(&self, name: &str) -> bool {
        matches!(&self.kind, Kind::Ident(s) if s == name)
    }
    fn is_punct(&self, c: char) -> bool {
        self.kind == Kind::Punct(c)
    }
    fn ident(&self) -> Option<&str> {
        match &self.kind {
            Kind::Ident(s) => Some(s),
            _ => None,
        }
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Rust source to identifiers, `::`, and punctuation. Comments, string and
/// char literals, lifetimes' quotes, and numbers produce nothing.
fn tokenize(src: &str) -> Vec<Tok> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let at = |j: usize| chars.get(j).copied().unwrap_or('\0');
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && at(i + 1) == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == '*' {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && at(i + 1) == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && at(i + 1) == '/' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if c == '"' {
            let end = skip_string(&chars, i + 1, &mut line);
            // A string that is exactly a path names what it points at:
            // `#[serde(default = "crate::ids::generate_short_uuid")]`.
            let body: String = chars[i + 1..end.saturating_sub(1)].iter().collect();
            if is_path_string(&body) {
                toks.extend(tokenize(&body).into_iter().map(|t| Tok { line, ..t }));
            }
            i = end;
        } else if c == '\'' {
            if at(i + 1) == '\\' {
                // Escaped char literal: '\n', '\'', '\u{..}'.
                i += 3;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if at(i + 2) == '\'' {
                i += 3;
            } else {
                // A lifetime; its name tokenizes as a harmless identifier.
                i += 1;
            }
        } else if c.is_ascii_digit() {
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
        } else if is_ident_start(c) {
            let start = i;
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            match (word.as_str(), at(i)) {
                ("r" | "br", '"' | '#')
                    if matches!(at(i), '"') || raw_string_follows(&chars, i) =>
                {
                    i = skip_raw_string(&chars, i, &mut line);
                }
                ("b", '"') => i = skip_string(&chars, i + 1, &mut line),
                ("b", '\'') => {
                    i += 1;
                    if at(i) == '\\' {
                        i += 1;
                    }
                    i += 2;
                }
                ("r", '#') if is_ident_start(at(i + 1)) => {
                    // Raw identifier `r#name`.
                    let s = i + 1;
                    i = s;
                    while i < chars.len() && is_ident_char(chars[i]) {
                        i += 1;
                    }
                    toks.push(Tok {
                        kind: Kind::Ident(chars[s..i].iter().collect()),
                        line,
                    });
                }
                _ => toks.push(Tok {
                    kind: Kind::Ident(word),
                    line,
                }),
            }
        } else if c == ':' && at(i + 1) == ':' {
            toks.push(Tok {
                kind: Kind::PathSep,
                line,
            });
            i += 2;
        } else {
            toks.push(Tok {
                kind: Kind::Punct(c),
                line,
            });
            i += 1;
        }
    }
    toks
}

fn is_path_string(s: &str) -> bool {
    (s.starts_with("crate::") || s.starts_with("super::"))
        && s.split("::").all(|seg| {
            let mut chars = seg.chars();
            chars.next().is_some_and(is_ident_start) && chars.all(is_ident_char)
        })
}

fn raw_string_follows(chars: &[char], mut i: usize) -> bool {
    while chars.get(i) == Some(&'#') {
        i += 1;
    }
    chars.get(i) == Some(&'"')
}

/// Past a `"`-delimited string body starting at `i`; returns the index after
/// the closing quote.
fn skip_string(chars: &[char], mut i: usize, line: &mut usize) -> usize {
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            '\n' => {
                *line += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    i
}

/// Past a raw string whose `#`s or opening quote start at `i`.
fn skip_raw_string(chars: &[char], mut i: usize, line: &mut usize) -> usize {
    let mut hashes = 0;
    while chars.get(i) == Some(&'#') {
        hashes += 1;
        i += 1;
    }
    i += 1; // opening quote
    while i < chars.len() {
        if chars[i] == '"' && (1..=hashes).all(|k| chars.get(i + k) == Some(&'#')) {
            return i + 1 + hashes;
        }
        if chars[i] == '\n' {
            *line += 1;
        }
        i += 1;
    }
    i
}

// ── Test code ───────────────────────────────────────────────────────────

/// Index just past the bracket group opening at `open`.
fn skip_group(toks: &[Tok], open: usize) -> usize {
    let mut depth = 0i32;
    let mut i = open;
    while i < toks.len() {
        match toks[i].kind {
            Kind::Punct('(' | '[' | '{') => depth += 1,
            Kind::Punct(')' | ']' | '}') => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    i
}

/// Whether the attribute whose `#` is at `i` is `#[cfg(test)]` or
/// `#[cfg(all(test, ..))]`, and the index just past it.
fn test_attr(toks: &[Tok], i: usize) -> Option<(bool, usize)> {
    if !toks[i].is_punct('#') {
        return None;
    }
    let open = i + 1;
    if !toks.get(open)?.is_punct('[') {
        return None;
    }
    let end = skip_group(toks, open);
    let body = &toks[open + 1..end - 1];
    let is_test = body.len() >= 4
        && body[0].is_ident("cfg")
        && body[1].is_punct('(')
        && (body[2].is_ident("test") && body[3].is_punct(')')
            || body[2].is_ident("all")
                && body[3].is_punct('(')
                && body.get(4).is_some_and(|t| t.is_ident("test")));
    Some((is_test, end))
}

/// Drops `#[cfg(test)]` items. Returns the remaining tokens and the names of
/// file modules declared test-only (`#[cfg(test)] mod tests;`). A file with
/// `#![cfg(test)]` comes back empty.
fn strip_test_items(toks: &[Tok]) -> (Vec<Tok>, Vec<String>) {
    if toks.len() > 5 && toks[0].is_punct('#') && toks[1].is_punct('!') {
        let inner: Vec<Tok> = std::iter::once(toks[0].clone())
            .chain(toks[2..].iter().cloned())
            .collect();
        if let Some((true, _)) = test_attr(&inner, 0) {
            return (Vec::new(), Vec::new());
        }
    }
    let mut out = Vec::new();
    let mut test_files = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let Some((true, mut j)) = test_attr(toks, i) else {
            out.push(toks[i].clone());
            i += 1;
            continue;
        };
        while let Some((_, next)) = test_attr(toks, j) {
            j = next;
        }
        // `mod name;` declares a file that is test code as a whole.
        let decl = toks[j..].iter().position(|t| t.is_ident("mod"));
        if let Some(k) = decl
            && toks[j..j + k].iter().all(|t| {
                t.is_ident("pub") || t.is_ident("crate") || t.is_punct('(') || t.is_punct(')')
            })
            && let Some(name) = toks.get(j + k + 1).and_then(Tok::ident)
            && toks.get(j + k + 2).is_some_and(|t| t.is_punct(';'))
        {
            test_files.push(name.to_string());
        }
        // Skip the item: to a `;` or the close of a `{}` at depth zero.
        let mut depth = 0i32;
        while j < toks.len() {
            match toks[j].kind {
                Kind::Punct('(' | '[' | '{') => depth += 1,
                Kind::Punct(')' | ']') => depth -= 1,
                Kind::Punct('}') => {
                    depth -= 1;
                    if depth == 0 {
                        j += 1;
                        break;
                    }
                }
                Kind::Punct(';') if depth == 0 => {
                    j += 1;
                    break;
                }
                _ => {}
            }
            j += 1;
        }
        i = j;
    }
    (out, test_files)
}

// ── Paths ───────────────────────────────────────────────────────────────

/// First segment of every entry in the `{..}` group opening at `open`,
/// descending through `internal::`.
fn group_heads(toks: &[Tok], open: usize, heads: &mut Vec<(String, usize)>) {
    let end = skip_group(toks, open);
    let mut i = open + 1;
    let mut entry_start = true;
    while i < end - 1 {
        if entry_start {
            if toks[i].is_ident("internal")
                && toks.get(i + 1).is_some_and(|t| t.kind == Kind::PathSep)
            {
                match toks.get(i + 2) {
                    Some(t) if t.is_punct('{') => group_heads(toks, i + 2, heads),
                    Some(t) => {
                        if let Some(name) = t.ident() {
                            heads.push((name.to_string(), t.line));
                        }
                    }
                    None => {}
                }
            } else if let Some(name) = toks[i].ident() {
                heads.push((name.to_string(), toks[i].line));
            }
            entry_start = false;
        }
        match toks[i].kind {
            Kind::Punct('{' | '(' | '[') => i = skip_group(toks, i),
            Kind::Punct(',') => {
                entry_start = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
}

/// Crate-root names (with line) that `toks` reaches, from `crate::` paths and
/// from `super::` chains that climb to the crate root or to `internal`.
/// `file_path` is the file's module path from the crate root.
fn crate_root_names(toks: &[Tok], file_path: &[String]) -> Vec<(String, usize)> {
    let mut names = Vec::new();
    let mut inline: Vec<(String, i32)> = Vec::new();
    let mut depth = 0i32;
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        let starts_path = i == 0 || toks[i - 1].kind != Kind::PathSep;
        if t.is_ident("mod")
            && let Some(name) = toks.get(i + 1).and_then(Tok::ident)
            && toks.get(i + 2).is_some_and(|t| t.is_punct('{'))
        {
            inline.push((name.to_string(), depth));
        }
        match t.kind {
            Kind::Punct('{') => depth += 1,
            Kind::Punct('}') => {
                depth -= 1;
                if inline.last().is_some_and(|(_, d)| *d == depth) {
                    inline.pop();
                }
            }
            _ => {}
        }
        if !starts_path {
            i += 1;
            continue;
        }
        // Where the path lands: `crate` is the root, each `super` climbs one.
        // `j` ends on the first segment after them.
        let sep = |k: usize| toks.get(k).is_some_and(|t| t.kind == Kind::PathSep);
        let (mut landing, mut j) = if t.is_ident("crate") && sep(i + 1) {
            (Vec::new(), i + 2)
        } else if t.is_ident("super") && sep(i + 1) {
            let mut path: Vec<String> = file_path.to_vec();
            path.extend(inline.iter().map(|(n, _)| n.clone()));
            let mut j = i;
            while toks.get(j).is_some_and(|t| t.is_ident("super")) && sep(j + 1) {
                path.pop();
                j += 2;
            }
            (path, j)
        } else {
            i += 1;
            continue;
        };
        if landing.is_empty()
            && toks.get(j).is_some_and(|t| t.is_ident("internal"))
            && toks.get(j + 1).is_some_and(|t| t.kind == Kind::PathSep)
        {
            landing.push("internal".to_string());
            j += 2;
        }
        let at_root = landing.is_empty() || landing == ["internal"];
        if at_root {
            match toks.get(j) {
                Some(n) if n.is_punct('{') => group_heads(toks, j, &mut names),
                Some(n) => {
                    if let Some(name) = n.ident() {
                        names.push((name.to_string(), n.line));
                    }
                }
                None => {}
            }
        }
        i = j;
    }
    names
}

// ── Walking the tree ────────────────────────────────────────────────────

struct SourceFile {
    path: PathBuf,
    /// Module path from the crate root, e.g. `["internal", "deck", "render"]`.
    module: Vec<String>,
    toks: Vec<Tok>,
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn module_path(src_root: &Path, file: &Path) -> Vec<String> {
    let rel = file.strip_prefix(src_root).expect("under src");
    let mut parts: Vec<String> = rel
        .iter()
        .map(|p| p.to_string_lossy().trim_end_matches(".rs").to_string())
        .collect();
    if parts.last().is_some_and(|p| p == "mod") {
        parts.pop();
    }
    parts
}

/// Production files under `dir`, test items stripped and test-only files dropped.
fn production_files(src_root: &Path, dir: &Path) -> Vec<SourceFile> {
    let mut paths = Vec::new();
    collect_rs(dir, &mut paths);
    paths.sort();
    let mut files = Vec::new();
    let mut test_modules: Vec<Vec<String>> = Vec::new();
    for path in paths {
        let src = std::fs::read_to_string(&path).expect("read source");
        let module = module_path(src_root, &path);
        let (toks, test_files) = strip_test_items(&tokenize(&src));
        for name in test_files {
            let mut m = module.clone();
            m.push(name);
            test_modules.push(m);
        }
        files.push(SourceFile { path, module, toks });
    }
    files.retain(|f| !test_modules.iter().any(|t| f.module.starts_with(t)));
    files
}

fn domain_modules(internal: &Path) -> BTreeSet<String> {
    std::fs::read_dir(internal)
        .expect("read src/internal")
        .map(|e| e.expect("dir entry").path())
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.to_string();
            if p.is_dir() {
                Some(name)
            } else {
                name.strip_suffix(".rs")
                    .filter(|n| *n != "mod")
                    .map(str::to_string)
            }
        })
        .collect()
}

/// Items `lib.rs` re-exports at the crate root from a domain module
/// (`pub use params::ShaderParams;`), by name, so `crate::ShaderParams` counts
/// as naming `params`.
fn root_reexports(src_root: &Path, modules: &BTreeSet<String>) -> BTreeMap<String, String> {
    let src = std::fs::read_to_string(src_root.join("lib.rs")).expect("read lib.rs");
    let toks = tokenize(&src);
    let mut out = BTreeMap::new();
    for (i, t) in toks.iter().enumerate() {
        if !(t.is_ident("pub") && toks.get(i + 1).is_some_and(|t| t.is_ident("use"))) {
            continue;
        }
        let Some(module) = toks.get(i + 2).and_then(Tok::ident) else {
            continue;
        };
        if !modules.contains(module) {
            continue;
        }
        let end = toks[i..]
            .iter()
            .position(|t| t.is_punct(';'))
            .map_or(toks.len(), |k| i + k);
        // The re-exported names: the last segment of each entry.
        let mut last = None;
        for t in &toks[i + 3..end] {
            match &t.kind {
                Kind::Ident(name) => last = Some(name.clone()),
                Kind::Punct(',' | '}') => {
                    if let Some(name) = last.take() {
                        out.insert(name, module.to_string());
                    }
                }
                _ => {}
            }
        }
        if let Some(name) = last {
            out.insert(name, module.to_string());
        }
    }
    out
}

/// The domain module a crate-root name refers to, if any.
fn resolve<'a>(
    name: &'a str,
    modules: &BTreeSet<String>,
    reexports: &'a BTreeMap<String, String>,
) -> Option<&'a str> {
    if modules.contains(name) {
        Some(name)
    } else {
        reexports.get(name).map(String::as_str)
    }
}

/// Every (from, to) domain edge with one example site.
fn domain_edges(src_root: &Path, modules: &BTreeSet<String>) -> BTreeMap<(String, String), String> {
    let reexports = root_reexports(src_root, modules);
    let mut edges = BTreeMap::new();
    for file in production_files(src_root, &src_root.join("internal")) {
        // `internal/mod.rs` itself belongs to no domain.
        let Some(from) = file.module.get(1).cloned() else {
            continue;
        };
        for (name, line) in crate_root_names(&file.toks, &file.module) {
            if let Some(to) = resolve(&name, modules, &reexports)
                && to != from
            {
                edges
                    .entry((from.clone(), to.to_string()))
                    .or_insert_with(|| format!("{}:{line}", file.path.display()));
            }
        }
    }
    edges
}

fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

#[test]
fn the_order_lists_every_domain_module() {
    let on_disk = domain_modules(&src_root().join("internal"));
    let listed: BTreeSet<String> = ORDER.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(listed.len(), ORDER.len(), "a module is listed twice");
    let unlisted: Vec<_> = on_disk.difference(&listed).collect();
    let missing: Vec<_> = listed.difference(&on_disk).collect();
    assert!(
        unlisted.is_empty() && missing.is_empty(),
        "ORDER must match src/internal/ (/spec/domain-dependencies.md). \
         Place new modules in their tier: {unlisted:?}. Remove deleted ones: {missing:?}"
    );
}

#[test]
fn domain_modules_depend_only_downward() {
    let root = src_root();
    let modules = domain_modules(&root.join("internal"));
    let rank: BTreeMap<&str, usize> = ORDER.iter().enumerate().map(|(i, m)| (*m, i)).collect();
    let upward: Vec<String> = domain_edges(&root, &modules)
        .into_iter()
        .filter(|((from, to), _)| rank.get(to.as_str()) > rank.get(from.as_str()))
        .map(|((from, to), site)| format!("{from} -> {to} (e.g. {site})"))
        .collect();
    assert!(
        upward.is_empty(),
        "a domain module names one above it in /spec/domain-dependencies.md. Move \
         the shared type or helper down, or pass the behavior in:\n{}",
        upward.join("\n")
    );
}

#[test]
fn engine_value_names_no_domain_module() {
    let root = src_root();
    let modules = domain_modules(&root.join("internal"));
    let reexports = root_reexports(&root, &modules);
    let mut found = Vec::new();
    for file in production_files(&root, &root.join("engine").join("value")) {
        for (name, line) in crate_root_names(&file.toks, &file.module) {
            if let Some(module) = resolve(&name, &modules, &reexports) {
                found.push(format!("{}:{line}: {module}", file.path.display()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "engine::value is the bottom of /spec/domain-dependencies.md and names no domain module:\n{}",
        found.join("\n")
    );
}

// ── The guard's own parsing ─────────────────────────────────────────────

fn names_in(src: &str, module: &[&str]) -> Vec<String> {
    let module: Vec<String> = module.iter().map(|s| (*s).to_string()).collect();
    let (toks, _) = strip_test_items(&tokenize(src));
    crate_root_names(&toks, &module)
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

#[test]
fn comments_strings_and_chars_name_nothing() {
    let src = concat!(
        "// crate::a::X\n",
        "/* crate::b::Y /* nested crate::c */ */\n",
        "/// [`crate::d`]\n",
        "const S: &str = \"see crate::e::Z\";\n",
        "const R: &str = r#\"crate::f \"quoted\"\"#;\n",
        "const C: char = '\"';\n",
        "const Q: char = '\\'';\n",
        "fn f<'a>(x: &'a u8) {}\n",
        "use crate::real::Thing;\n",
    );
    assert_eq!(names_in(src, &["internal", "m"]), ["real"]);
}

#[test]
fn a_string_that_is_a_path_names_its_module() {
    let src = r#"#[serde(default = "crate::ids::fresh")] id: String, msg: &'static str = "crate::no thanks";"#;
    assert_eq!(names_in(src, &["internal", "m"]), ["ids"]);
}

#[test]
fn use_groups_and_internal_paths_expand() {
    let src = "use crate::{a::X, b, internal::{c, d::Y}}; fn f() { crate::internal::e::g(); }";
    assert_eq!(names_in(src, &["internal", "m"]), ["a", "b", "c", "d", "e"]);
}

#[test]
fn super_chains_resolve_from_the_file_and_inline_modules() {
    // From internal/m/sub.rs, `super::super` is `internal`, one more is the root.
    let src = "use super::super::sibling::X; use super::super::super::root_item; use super::local;";
    assert_eq!(
        names_in(src, &["internal", "m", "sub"]),
        ["sibling", "root_item"]
    );
    let src = "mod inner { use super::super::other; }";
    assert_eq!(names_in(src, &["internal", "m"]), ["other"]);
}

#[test]
fn test_items_and_files_are_dropped() {
    let src = r#"
        #[cfg(test)]
        mod tests { use crate::fixture::F; }
        #[cfg(all(test, feature = "x"))]
        fn helper() { crate::other::g(); }
        #[cfg(test)]
        mod file_tests;
        #[cfg(feature = "x")]
        use crate::kept::K;
    "#;
    let (toks, test_files) = strip_test_items(&tokenize(src));
    assert_eq!(test_files, ["file_tests"]);
    let module = vec!["internal".to_string(), "m".to_string()];
    let names: Vec<String> = crate_root_names(&toks, &module)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names, ["kept"]);
}
