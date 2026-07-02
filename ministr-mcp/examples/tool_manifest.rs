//! Print the full MCP tool manifest as JSON.
//!
//! Regenerates the docs-parity manifest:
//!
//! ```sh
//! cargo run -p ministr-mcp --example tool_manifest > docs/reference/tools-manifest.json
//! ```

fn main() {
    print!("{}", ministr_mcp::server::manifest::tool_manifest_pretty());
}
