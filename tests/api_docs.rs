//! Generates the route reference in `docs/13-api.md` from `ApiDoc::openapi()`
//! and fails when the committed file has drifted.
//!
//! Hand-maintained route docs cannot be kept honest — see
//! `/spec/api-addressing.md` § Documentation generation. The generated block
//! lives between the `BEGIN GENERATED ROUTES` / `END GENERATED ROUTES` markers;
//! prose outside them is preserved.
//!
//! To regenerate after changing routes:
//!
//! ```sh
//! UPDATE_API_DOCS=1 cargo test --test api_docs
//! ```

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use utoipa::OpenApi;

const BEGIN: &str = "<!-- BEGIN GENERATED ROUTES -->";
const END: &str = "<!-- END GENERATED ROUTES -->";

/// Methods in the order they should be listed for a path.
const METHOD_ORDER: [&str; 6] = ["get", "post", "put", "patch", "delete", "head"];

struct Operation {
    method: String,
    path: String,
    summary: String,
}

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn method_rank(method: &str) -> usize {
    METHOD_ORDER
        .iter()
        .position(|m| m.eq_ignore_ascii_case(method))
        .unwrap_or(usize::MAX)
}

/// Every documented operation, grouped by its first OpenAPI tag.
fn operations_by_tag() -> BTreeMap<String, Vec<Operation>> {
    let json = serde_json::to_value(varda::usecases::api::runner::ApiDoc::openapi())
        .expect("serialize openapi");
    let paths = json
        .get("paths")
        .and_then(|p| p.as_object())
        .expect("openapi paths")
        .clone();

    let mut by_tag: BTreeMap<String, Vec<Operation>> = BTreeMap::new();
    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        for method in METHOD_ORDER {
            let Some(op) = item.get(method) else {
                continue;
            };
            let summary = op
                .get("summary")
                .and_then(|s| s.as_str())
                .or_else(|| op.get("description").and_then(|d| d.as_str()))
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            let tag = op
                .get("tags")
                .and_then(|t| t.as_array())
                .and_then(|t| t.first())
                .and_then(|t| t.as_str())
                .unwrap_or("Other")
                .to_string();
            by_tag.entry(tag).or_default().push(Operation {
                method: method.to_uppercase(),
                path: path.clone(),
                summary,
            });
        }
    }
    for group in by_tag.values_mut() {
        group.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then_with(|| method_rank(&a.method).cmp(&method_rank(&b.method)))
        });
    }
    by_tag
}

fn render_reference() -> String {
    let mut out = String::new();
    out.push_str(BEGIN);
    out.push_str("\n\n");
    out.push_str(
        "<!-- Generated from ApiDoc::openapi() by tests/api_docs.rs.\n     \
         Regenerate with: UPDATE_API_DOCS=1 cargo test --test api_docs -->\n\n",
    );
    out.push_str(
        "Writes address entities by UUID. Positional integers appear only as reorder\n\
         ordinals and sequence step indices — see [/spec/api-addressing.md].\n",
    );

    for (tag, group) in operations_by_tag() {
        out.push_str(&format!("\n### {}\n\n", tag));
        out.push_str("| Method | Path | Description |\n|---|---|---|\n");
        for op in group {
            out.push_str(&format!(
                "| `{}` | `{}` | {} |\n",
                op.method, op.path, op.summary
            ));
        }
    }

    out.push('\n');
    out.push_str(END);
    out
}

fn splice(existing: &str, generated: &str) -> String {
    let begin = existing
        .find(BEGIN)
        .unwrap_or_else(|| panic!("docs/13-api.md is missing the `{BEGIN}` marker"));
    let end = existing
        .find(END)
        .unwrap_or_else(|| panic!("docs/13-api.md is missing the `{END}` marker"))
        + END.len();
    format!("{}{}{}", &existing[..begin], generated, &existing[end..])
}

#[test]
fn route_reference_is_up_to_date() {
    let path = manifest_path("docs/13-api.md");
    let existing = std::fs::read_to_string(&path).expect("read docs/13-api.md");
    let updated = splice(&existing, &render_reference());

    if existing == updated {
        return;
    }
    if std::env::var_os("UPDATE_API_DOCS").is_some() {
        std::fs::write(&path, updated).expect("write docs/13-api.md");
        return;
    }
    panic!(
        "docs/13-api.md route reference is out of date with the OpenAPI spec.\n\
         Regenerate with: UPDATE_API_DOCS=1 cargo test --test api_docs"
    );
}

/// Every path passed to `Router::route`, whether the call is on one line or
/// wrapped across several by rustfmt.
fn registered_routes(src: &str) -> Vec<String> {
    let mut routes = Vec::new();
    let mut rest = src;
    while let Some(pos) = rest.find(".route(") {
        rest = &rest[pos + ".route(".len()..];
        // The path is the next string literal, possibly after a line break.
        let Some(open) = rest.find('"') else { break };
        if rest[..open].contains(';') {
            continue; // not a route registration after all
        }
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else { break };
        routes.push(after[..close].to_string());
        rest = &after[close..];
    }
    routes
}

/// The generated reference is only honest if every registered route is actually
/// documented. A route present in `runner.rs` but absent from the `utoipa`
/// derive would silently never appear in the docs.
#[test]
fn every_registered_route_is_documented() {
    let runner = std::fs::read_to_string(manifest_path("src/usecases/api/runner.rs"))
        .expect("read runner.rs");

    let documented: HashSet<String> = operations_by_tag()
        .into_values()
        .flatten()
        .map(|op| op.path)
        .collect();

    // Served for reasons other than the documented command surface: the Swagger
    // UI itself, the WebSocket upgrade, and the static control panel. None of
    // these are OpenAPI operations.
    let exempt = ["/api/ws", "/api/docs", "/api/openapi.json", "/"];

    let mut undocumented = Vec::new();
    for route in registered_routes(&runner) {
        if exempt.contains(&route.as_str()) || documented.contains(&route) {
            continue;
        }
        undocumented.push(format!("`{}`", route));
    }
    undocumented.sort();
    undocumented.dedup();

    assert!(
        undocumented.is_empty(),
        "every registered route needs a documented utoipa operation so the \
         generated reference stays complete (/spec/api-addressing.md); found:\n{}",
        undocumented.join("\n")
    );
}
