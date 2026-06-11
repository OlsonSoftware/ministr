# Tool Guide

## Codebase Navigation (ministr)

| Tool | Purpose |
|------|---------|
| `ministr_survey` | Semantic search across docs and code. Start here. |
| `ministr_symbols` | Find structs, functions, traits, enums by name/kind/module. |
| `ministr_definition` | Get full source of a symbol by ID. |
| `ministr_references` | Find callers, implementors, importers of a symbol. |
| `ministr_read` | Read a section by ID (with deduplication and delta delivery). |
| `ministr_extract` | Get atomic claims from a section, optionally filtered by query. |
| `ministr_toc` | Structural overview of the indexed corpus. |
| `ministr_bridge` | Cross-language bridge links. **Use before changing any IPC/FFI boundary.** |

Recommended workflow: `ministr_survey` → `ministr_symbols` → `ministr_definition` / `ministr_read` → dig deeper with `ministr_references` / `ministr_bridge`.

See `ministr-playbook.md` for decision trees and chaining patterns.

## Tool Preferences

- Use `ministr_survey` instead of Glob/find for file and concept discovery.
- Use `ministr_symbols` instead of Grep for finding code symbols.
- Use ministr tools for exploration; `Read` only immediately before `Edit`.
