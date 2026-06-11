# ministr MCP — Codebase Navigation

ministr is the **required** tool for all codebase exploration. Do NOT use built-in search tools.

## Tool Rules

| Tool                              | Status         | Usage                                                                         |
| --------------------------------- | -------------- | ----------------------------------------------------------------------------- |
| `ministr_survey(query: "...")`       | **PRIMARY**    | Semantic search across docs and code. Start here.                             |
| `ministr_symbols(query: "...")`      | **PRIMARY**    | Find structs, functions, traits, enums by name/kind/module.                   |
| `ministr_definition(id: "...")`      | **PRIMARY**    | Get full source of a symbol by ID.                                            |
| `ministr_references(id: "...")`      | **PRIMARY**    | Find callers, implementors, importers of a symbol.                            |
| `ministr_read(id: "...")`            | **PRIMARY**    | Read a section by ID (with deduplication and delta delivery).                 |
| `ministr_extract(id: "...")`         | **PRIMARY**    | Get atomic claims from a section, optionally filtered by query.               |
| `ministr_toc`                        | **PRIMARY**    | Structural overview of the indexed corpus.                                    |
| `ministr_bridge(query/kind/...)`     | **PRIMARY**    | Cross-language bridge links (Tauri, PyO3, NAPI, etc.).                        |
| `Grep` / `Glob`                   | **BLOCKED**    | Denied by PreToolUse hook. Use ministr_survey or ministr_symbols instead.           |
| `Bash(grep/rg/find/...)`          | **BLOCKED**    | Denied by PreToolUse hook. Do NOT shell out for search or file discovery.     |
| `Bash(... \| grep/head/tail/wc)`  | **BLOCKED**    | Denied by PreToolUse hook. Do NOT pipe to search/filter tools.               |
| `Read(file)`                      | **RESTRICTED** | Use `Read` only immediately before `Edit`. Never for exploration.             |

## Prohibited Patterns

These are **hard-blocked** by PreToolUse hooks and will be denied:

- `grep`, `rg`, `ag`, `ack`, `egrep`, `fgrep` — use `ministr_survey` instead
- `find`, `fd` — use `ministr_toc` or `ministr_survey` instead
- `cat file | grep`, `cmd | head`, `cmd | tail`, `cmd | wc` — use ministr tools instead
- `Grep(pattern)`, `Glob(pattern)` — use `ministr_survey` or `ministr_symbols` instead

## Workflow

1. **`ministr_survey` first** — semantic search across docs and code. Always start here.
2. **`ministr_symbols` for code navigation** — find symbols by name, kind, or module.
3. **`ministr_definition` / `ministr_read`** — get full source of a symbol or section.
4. **`ministr_references` before modifying shared code** — find callers, implementors, importers.
5. **`ministr_bridge` before modifying any cross-language boundary** — see all endpoints.
6. **`ministr_toc`** — structural overview when you need to understand project layout.

See `ministr-playbook.md` for detailed decision trees and chaining patterns.
