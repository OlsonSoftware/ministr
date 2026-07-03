# ministr-mcp

The MCP server adapter: routes JSON-RPC via the `rmcp` crate, registers the
tool surface, maps requests onto `ministr-core` service traits, and tracks
per-session delivery state (dedup, delta delivery, coherence alerts).

The tool surface is self-documenting by construction: `src/server/manifest.rs`
exposes the full tool manifest, `docs/reference/tools/` is generated from it
(`cargo run -p ministr-mcp --example gen_tool_docs`), and
`tests/tool_manifest_parity.rs` fails when the committed manifest or pages
drift from the code.

Place in the workspace: see the
[architecture overview](../docs/concepts/architecture.md).
