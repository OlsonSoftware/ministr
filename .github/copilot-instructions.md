# ministr MCP — Codebase Navigation

This project uses [ministr](https://github.com/OlsonSoftware/ministr) as an MCP server for semantic code search. ministr is the **preferred** tool for codebase *exploration*; it does not restrict normal shell work.

## Policy

- Prefer ministr MCP tools for code discovery, search, and navigation.
- The built-in **Grep** / **Glob** tools are not for exploration here —
  use `ministr_survey` / `ministr_toc`.
- The shell is unrestricted: building, testing, dependency installs,
  `git`, running the project, and filtering command output
  (`cargo test | grep`, `cargo build 2>&1 | tail`, `git log | grep`)
  all run normally. A *leading* `grep`/`find` is auto-allowed with a
  one-line hint to prefer ministr — it never prompts.
- Read files only immediately before editing them.

## Tool Mapping (preferences, not prohibitions)

| For… | Prefer… |
|------|---------|
| code / text search | `ministr_survey(query: "...")` — semantic search across docs and code |
| file / structure discovery | `ministr_toc` — structural overview of the indexed corpus |
| finding a symbol | `ministr_symbols(query: "name")` — by name/kind/module |
| a specific function's source | `ministr_definition(symbol_id: "...")` |
| who calls a function | `ministr_references(symbol_id: "...")` |

## Workflow

1. **Start with `ministr_survey`** for any question about the codebase
2. **Use `ministr_symbols`** to find specific code symbols
3. **Use `ministr_definition` or `ministr_read`** to get full source
4. **Use `ministr_references`** before modifying shared code (find all callers)
5. **Use `ministr_bridge`** before modifying cross-language boundaries (Tauri, FFI, etc.)
