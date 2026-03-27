# iris MCP — Mandatory Codebase Navigation

iris is the **required** tool for all codebase exploration. Do NOT use Glob, Grep, or Read for discovery.

## Tool Rules

| Tool                              | Status         | Usage                                                                             |
| --------------------------------- | -------------- | --------------------------------------------------------------------------------- |
| `iris_survey(query: "...")`       | **MANDATORY**  | Semantic search across docs and code. Start here.                                 |
| `iris_symbols(query: "...")`      | **MANDATORY**  | Find structs, functions, traits, enums by name/kind/module.                       |
| `iris_definition(id: "...")`      | **MANDATORY**  | Get full source of a symbol by ID.                                                |
| `iris_references(id: "...")`      | **MANDATORY**  | Find callers, implementors, importers of a symbol.                                |
| `iris_read(id: "...")`            | **MANDATORY**  | Read a section by ID (with deduplication and delta delivery).                     |
| `iris_extract(id: "...")`         | **MANDATORY**  | Get atomic claims from a section, optionally filtered by query.                   |
| `iris_toc`                        | **MANDATORY**  | Structural overview of the indexed corpus.                                        |
| `iris_bridge(query/kind/...)`     | **MANDATORY**  | Cross-language bridge links.                                                      |
| `Read(file)`                      | **RESTRICTED** | ONLY allowed as the required step immediately before Edit. Never for exploration. |

## Workflow

1. **`iris_survey` first** — semantic search across docs and code. Always start here.
2. **`iris_symbols` for code navigation** — find symbols by name, kind, or module.
3. **`iris_definition` / `iris_read`** — get full source of a symbol or section.
4. **`iris_references` before modifying shared code** — find callers, implementors, importers.
5. **`iris_toc`** — structural overview when you need to understand project layout.

## Testing iris Changes

**NEVER spin up a second iris instance against this repo.** The iris MCP server is already running on this codebase. A second instance causes conflicts — shared SQLite, shared HNSW indexes, shared session state.

- Using the live MCP tools in a session is fine — that's what they're for
- After implementing changes, run `cargo install --path iris-cli` to rebuild, then ask the user to restart
- For automated tests: use `cargo test` with `tempdir()` fixtures
