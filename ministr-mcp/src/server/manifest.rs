//! Tool-manifest introspection — the docs-parity seam.
//!
//! Exposes the complete, **unpruned** MCP tool surface (every tool the
//! `#[tool_router]` macro registers, before any runtime `prune_tools`
//! hiding) as stable, sorted JSON. The docs site's tool reference is
//! generated from this manifest, so the parameter tables users read are
//! the same schemas agents receive — by construction, not by review.
//!
//! Two gates consume it:
//! - `tests/tool_manifest_parity.rs` fails `cargo test` when the committed
//!   `web/content/tools-manifest.json` no longer matches the code;
//! - `web/scripts/gen-tool-docs.mjs --check` fails the web build when the
//!   reference pages no longer match the manifest.
//!
//! Regenerate with: `cargo run -p ministr-mcp --example tool_manifest`.

use super::MinistrServer;

/// The full tool surface as a JSON array, sorted by tool name.
///
/// Each entry: `{ "name", "description", "input_schema", "annotations"? }`,
/// exactly as served in `tools/list` (modulo runtime pruning).
#[must_use]
pub fn tool_manifest() -> serde_json::Value {
    let mut tools = MinistrServer::tool_router().list_all();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    serde_json::Value::Array(
        tools
            .into_iter()
            .map(|t| {
                let mut entry = serde_json::Map::new();
                entry.insert("name".into(), t.name.as_ref().into());
                entry.insert(
                    "description".into(),
                    t.description
                        .as_deref()
                        .map_or(serde_json::Value::Null, Into::into),
                );
                entry.insert(
                    "input_schema".into(),
                    serde_json::Value::Object((*t.input_schema).clone()),
                );
                if let Some(annotations) = &t.annotations
                    && let Ok(a) = serde_json::to_value(annotations)
                {
                    entry.insert("annotations".into(), a);
                }
                serde_json::Value::Object(entry)
            })
            .collect(),
    )
}

/// Pretty-printed [`tool_manifest`] with a trailing newline — the exact
/// byte content of the committed manifest file.
#[must_use]
pub fn tool_manifest_pretty() -> String {
    let mut s = serde_json::to_string_pretty(&tool_manifest()).unwrap_or_else(|_| "[]".into());
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_lists_the_full_unpruned_surface() {
        let manifest = tool_manifest();
        let arr = manifest.as_array().expect("array");
        assert!(
            arr.len() >= 25,
            "expected the full tool surface (>=25), got {}",
            arr.len()
        );
        // Sorted + unique names, every entry carries a schema.
        let names: Vec<&str> = arr
            .iter()
            .map(|t| t["name"].as_str().expect("name"))
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted, "manifest must be sorted and duplicate-free");
        for t in arr {
            assert!(
                t["input_schema"].is_object(),
                "{} missing schema",
                t["name"]
            );
            assert!(
                t["description"].is_string(),
                "{} missing description",
                t["name"]
            );
        }
    }
}
