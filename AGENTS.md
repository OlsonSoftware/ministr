# Agent Instructions

This repository is ministr itself — a local code intelligence MCP server.
It uses its own tooling: ministr is configured via `.mcp.json` (Claude
Code), `.vscode/mcp.json` (VS Code / Copilot), and `.cursor/mcp.json`
(Cursor), and is the preferred tool for codebase *exploration*. It does not
restrict normal shell work.

## Exploration tools

The full 25-tool surface is documented in the
[tool reference](docs/reference/tools/README.md). The core loop:

| Tool | Purpose |
|------|---------|
| `ministr_survey(query)` | Semantic search across docs and code. Start here. |
| `ministr_symbols(query)` | Find functions, structs, traits, enums by name/kind/module. |
| `ministr_definition(symbol_id)` | Full source of a symbol. |
| `ministr_references(symbol_id)` | Callers, implementors, importers — check before modifying. |
| `ministr_read(section_id)` | Full content of a section. |
| `ministr_toc` | Structural overview of the corpus. |
| `ministr_bridge(query)` | Cross-language links — check before touching the Tauri boundary. |
| `ministr_diagnostics` | Structured toolchain diagnostics (cargo, tsc, …). |

## Policy (preferences, not prohibitions)

- Prefer ministr for code discovery, search, and navigation; the built-in
  Grep/Glob tools are not for exploration here.
- The shell is unrestricted: building, testing, `git`, installing
  dependencies, and filtering command output all run normally.
- Read files only immediately before editing them.

## Workflow

1. `ministr_survey` → understand concepts, find relevant code
2. `ministr_symbols` → locate specific symbols
3. `ministr_definition` / `ministr_read` → get full source
4. `ministr_references` → check impact before modifying
5. `ministr_bridge` → check cross-language boundaries (the desktop app is
   Tauri: Rust ↔ TypeScript)
6. Only then: `Read` → `Edit`

## Verifying changes

The canonical gate is `just validate` (fmt-check, clippy with pedantic
warnings denied, the full test suite, app typecheck + build, black-box
guard). A change is not done until it passes. Two docs gates live inside
`cargo test`: the committed tool manifest and the generated blocks in
`docs/reference/tools/` must match the code — regenerate with
`cargo run -p ministr-mcp --example tool_manifest > docs/reference/tools-manifest.json`
and `cargo run -p ministr-mcp --example gen_tool_docs` after changing any
tool's name, description, or schema.

## Documentation

User-facing docs live in [docs/](docs/README.md). Pages under
`docs/reference/tools/` are generated — edit prose outside the `@generated`
markers only.
