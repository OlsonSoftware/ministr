//! Docs-parity gate: the committed tool manifest must match the code.
//!
//! When this fails, a tool's name/description/schema changed without the
//! docs manifest being regenerated. Fix with:
//!
//! ```sh
//! cargo run -p ministr-mcp --example tool_manifest > web/content/tools-manifest.json
//! cd web && npm run docs:gen
//! ```

use std::path::Path;

#[test]
fn committed_tool_manifest_matches_code() {
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/content/tools-manifest.json");
    let committed = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
        panic!(
            "missing {} ({e}) — generate it: \
             cargo run -p ministr-mcp --example tool_manifest > web/content/tools-manifest.json",
            manifest_path.display()
        )
    });
    let live = ministr_mcp::server::manifest::tool_manifest_pretty();
    assert_eq!(
        committed, live,
        "web/content/tools-manifest.json is stale — the tool surface changed. \
         Regenerate: cargo run -p ministr-mcp --example tool_manifest > \
         web/content/tools-manifest.json && (cd web && npm run docs:gen)"
    );
}
