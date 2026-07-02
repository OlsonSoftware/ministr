//! Generate the tool reference pages under `docs/reference/tools/`.
//!
//! For each tool: creates the page if missing, otherwise replaces only the
//! `@generated` block — hand-written prose outside the markers survives.
//! Also rewrites the fully-generated `README.md` index.
//!
//! ```sh
//! cargo run -p ministr-mcp --example gen_tool_docs
//! ```

use ministr_mcp::server::manifest::{
    DOC_BLOCK_END, DOC_BLOCK_START, tool_doc_blocks, tool_index_markdown,
};
use std::fs;
use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/reference/tools");
    fs::create_dir_all(&dir).expect("create docs/reference/tools");

    for tool in tool_doc_blocks() {
        let page = dir.join(format!("{}.md", tool.slug));
        let content = if page.exists() {
            let existing = fs::read_to_string(&page).expect("read page");
            let start = existing
                .find(DOC_BLOCK_START)
                .unwrap_or_else(|| panic!("{}: missing start marker", page.display()));
            let end = existing
                .find(DOC_BLOCK_END)
                .unwrap_or_else(|| panic!("{}: missing end marker", page.display()));
            format!(
                "{}{}{}",
                &existing[..start],
                tool.block,
                &existing[end + DOC_BLOCK_END.len()..]
            )
        } else {
            format!("# {}\n\n{}\n", tool.name, tool.block)
        };
        fs::write(&page, content).expect("write page");
        println!("wrote {}", page.display());
    }

    let index = dir.join("README.md");
    fs::write(&index, tool_index_markdown()).expect("write index");
    println!("wrote {}", index.display());
}
