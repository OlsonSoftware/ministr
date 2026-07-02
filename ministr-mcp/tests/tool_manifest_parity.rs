//! Docs-parity gates: the committed tool manifest and the generated blocks
//! in the tool reference pages must match the code.
//!
//! When these fail, a tool's name/description/schema changed without the
//! docs being regenerated. Fix with:
//!
//! ```sh
//! cargo run -p ministr-mcp --example tool_manifest > docs/reference/tools-manifest.json
//! cargo run -p ministr-mcp --example gen_tool_docs
//! ```

use ministr_mcp::server::manifest;
use std::collections::BTreeSet;
use std::path::Path;

const REGEN: &str = "Regenerate: cargo run -p ministr-mcp --example tool_manifest > \
     docs/reference/tools-manifest.json && cargo run -p ministr-mcp --example gen_tool_docs";

#[test]
fn committed_tool_manifest_matches_code() {
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/reference/tools-manifest.json");
    let committed = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
        panic!("missing {} ({e}) — {REGEN}", manifest_path.display());
    });
    let live = manifest::tool_manifest_pretty();
    assert_eq!(
        committed, live,
        "docs/reference/tools-manifest.json is stale — the tool surface changed. {REGEN}"
    );
}

#[test]
fn committed_tool_pages_match_code() {
    let tools_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/reference/tools");
    let blocks = manifest::tool_doc_blocks();
    let mut expected_files: BTreeSet<String> =
        blocks.iter().map(|t| format!("{}.md", t.slug)).collect();
    expected_files.insert("README.md".into());

    for tool in &blocks {
        let page = tools_dir.join(format!("{}.md", tool.slug));
        let content = std::fs::read_to_string(&page).unwrap_or_else(|e| {
            panic!("missing {} ({e}) — {REGEN}", page.display());
        });
        assert!(
            content.contains(&tool.block),
            "{}: generated block is stale for {}. {REGEN}",
            page.display(),
            tool.name
        );
    }

    let index = std::fs::read_to_string(tools_dir.join("README.md"))
        .unwrap_or_else(|e| panic!("missing tools index README.md ({e}) — {REGEN}"));
    assert_eq!(
        index,
        manifest::tool_index_markdown(),
        "docs/reference/tools/README.md is stale. {REGEN}"
    );

    let on_disk: BTreeSet<String> = std::fs::read_dir(&tools_dir)
        .expect("read docs/reference/tools")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            Path::new(n)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        })
        .collect();
    let orphans: Vec<&String> = on_disk.difference(&expected_files).collect();
    assert!(
        orphans.is_empty(),
        "orphan pages in docs/reference/tools (no matching tool): {orphans:?}"
    );
}
