//! Tool-manifest introspection — the docs-parity seam.
//!
//! Exposes the complete, **unpruned** MCP tool surface (every tool the
//! `#[tool_router]` macro registers, before any runtime `prune_tools`
//! hiding) as stable, sorted JSON. The docs site's tool reference is
//! generated from this manifest, so the parameter tables users read are
//! the same schemas agents receive — by construction, not by review.
//!
//! Two gates in `tests/tool_manifest_parity.rs` consume it: one fails
//! `cargo test` when the committed `docs/reference/tools-manifest.json` no
//! longer matches the code, the other when the generated blocks in the
//! `docs/reference/tools/` pages drift from the live schemas.
//!
//! Regenerate both with:
//!
//! ```sh
//! cargo run -p ministr-mcp --example tool_manifest > docs/reference/tools-manifest.json
//! cargo run -p ministr-mcp --example gen_tool_docs
//! ```

use super::MinistrServer;

/// Opening marker of the generated block in a tool reference page.
/// Everything between the markers is machine-owned; prose outside them is
/// hand-written and preserved across regeneration.
pub const DOC_BLOCK_START: &str = "<!-- @generated tool-docs start — do not edit this block; \
     regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->";

/// Closing marker of the generated block in a tool reference page.
pub const DOC_BLOCK_END: &str = "<!-- @generated tool-docs end -->";

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

/// One tool's rendered reference-page material.
#[derive(Debug, Clone)]
pub struct ToolDocBlock {
    /// The tool name as agents see it (e.g. `ministr_survey`).
    pub name: String,
    /// Page file stem (e.g. `survey` for `ministr_survey`).
    pub slug: String,
    /// The full generated block, markers included.
    pub block: String,
}

/// The generated block for every tool, sorted by tool name.
///
/// Pages under `docs/reference/tools/` embed these blocks between
/// [`DOC_BLOCK_START`]/[`DOC_BLOCK_END`]; `tests/tool_manifest_parity.rs`
/// fails when a committed page's block differs from this output.
#[must_use]
pub fn tool_doc_blocks() -> Vec<ToolDocBlock> {
    let manifest = tool_manifest();
    let tools = manifest.as_array().cloned().unwrap_or_default();
    tools
        .iter()
        .map(|tool| {
            let name = tool["name"].as_str().unwrap_or_default().to_string();
            ToolDocBlock {
                slug: slug_of(&name),
                block: block_for(tool),
                name,
            }
        })
        .collect()
}

/// The full content of the generated `docs/reference/tools/README.md` index.
#[must_use]
pub fn tool_index_markdown() -> String {
    use std::fmt::Write as _;
    let mut out = String::from(
        "# Tool reference\n\n\
         <!-- @generated tool-index — do not edit; \
         regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->\n\n\
         Generated from the MCP tool manifest — the same schemas agents receive.\n\n\
         | Tool | Description |\n|---|---|\n",
    );
    let manifest = tool_manifest();
    for tool in manifest.as_array().into_iter().flatten() {
        let name = tool["name"].as_str().unwrap_or_default();
        let first = first_sentence(tool["description"].as_str().unwrap_or_default());
        let _ = writeln!(
            out,
            "| [`{name}`]({}.md) | {} |",
            slug_of(name),
            cell(&first)
        );
    }
    out
}

/// Page file stem for a tool name: `ministr_` stripped, `_` → `-`.
fn slug_of(name: &str) -> String {
    name.trim_start_matches("ministr_").replace('_', "-")
}

/// First sentence of a description (up to the first `. `), newlines collapsed.
fn first_sentence(text: &str) -> String {
    let flat = text.replace('\n', " ");
    match flat.find(". ") {
        Some(i) => flat[..=i].to_string(),
        None => flat,
    }
}

/// Escape `<`/`>` outside backtick code spans so descriptions render as
/// text, not HTML, in GitHub markdown.
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, part) in text.split('`').enumerate() {
        if i > 0 {
            out.push('`');
        }
        if i % 2 == 1 {
            out.push_str(part);
        } else {
            for c in part.chars() {
                if c == '<' || c == '>' {
                    out.push('\\');
                }
                out.push(c);
            }
        }
    }
    out
}

/// Table-cell form of a description: escaped, pipes escaped, single line.
fn cell(text: &str) -> String {
    escape_text(text).replace('|', "\\|").replace('\n', " ")
}

/// Human-readable type for a schema property, ported from the retired
/// `gen-tool-docs.mjs` so rendered tables stay stable.
fn type_of(prop: &serde_json::Value) -> String {
    let mut t: Option<String> = match &prop["type"] {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(a) => {
            let parts: Vec<&str> = a
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|s| *s != "null")
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" | "))
            }
        }
        _ => None,
    };
    if t.as_deref() == Some("array") {
        match &prop["items"]["type"] {
            serde_json::Value::String(item) => t = Some(format!("array of {item}")),
            serde_json::Value::Array(items) => {
                let joined: Vec<&str> =
                    items.iter().filter_map(serde_json::Value::as_str).collect();
                if !joined.is_empty() {
                    t = Some(format!("array of {}", joined.join("/")));
                }
            }
            _ => {}
        }
    }
    if t.is_none()
        && let Some(any_of) = prop["anyOf"].as_array()
    {
        let parts: Vec<&str> = any_of
            .iter()
            .map(|v| v["type"].as_str().unwrap_or("…"))
            .filter(|s| *s != "null")
            .collect();
        if !parts.is_empty() {
            t = Some(parts.join(" | "));
        }
    }
    t.unwrap_or_else(|| "any".into())
}

/// Render one tool's generated block, markers included.
fn block_for(tool: &serde_json::Value) -> String {
    let schema = &tool["input_schema"];
    let empty = serde_json::Map::new();
    let props = schema["properties"].as_object().unwrap_or(&empty);
    let required_list: Vec<&str> = schema["required"]
        .as_array()
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();

    let mut lines = vec![DOC_BLOCK_START.to_string(), String::new()];
    lines.push(format!(
        "> {}",
        escape_text(tool["description"].as_str().unwrap_or_default())
    ));
    lines.push(String::new());
    lines.push("## Parameters".into());
    lines.push(String::new());
    let mut names: Vec<&String> = props.keys().collect();
    names.sort();
    if names.is_empty() {
        lines.push("None.".into());
    } else {
        lines.push("| Parameter | Type | Required | Description |".into());
        lines.push("|---|---|---|---|".into());
        for name in names {
            let p = &props[name];
            let nullable = p["type"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x == "null"));
            let required =
                required_list.contains(&name.as_str()) || (!nullable && p["default"].is_null());
            lines.push(format!(
                "| `{name}` | {} | {} | {} |",
                type_of(p),
                if required { "yes" } else { "no" },
                cell(p["description"].as_str().unwrap_or_default())
            ));
        }
    }
    let a = &tool["annotations"];
    let mut hints = Vec::new();
    if a["readOnlyHint"] == true {
        hints.push("read-only");
    }
    if a["destructiveHint"] == true {
        hints.push("destructive");
    }
    if a["idempotentHint"] == true {
        hints.push("idempotent");
    }
    if a["openWorldHint"] == true {
        hints.push("open-world");
    }
    if !hints.is_empty() {
        lines.push(String::new());
        lines.push(format!("Annotations: {}.", hints.join(" · ")));
    }
    lines.push(String::new());
    lines.push(
        "<small>This block is generated from the live tool schema — the same definition \
         agents receive.</small>"
            .into(),
    );
    lines.push(String::new());
    lines.push(DOC_BLOCK_END.to_string());
    lines.join("\n")
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
