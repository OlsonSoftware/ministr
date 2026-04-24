# ministr MCP — Codebase Navigation (MANDATORY)

This project uses [ministr](https://github.com/OlsonSoftware/ministr) as an MCP server for semantic code search.

## CRITICAL: Tool Restrictions

**You MUST use ministr MCP tools for ALL codebase exploration.** The following are PROHIBITED:

### Prohibited Tools and Patterns

- ❌ **Grep tool** — use `ministr_survey(query: "...")` instead
- ❌ **Glob tool** — use `ministr_toc` instead
- ❌ **`grep`**, **`rg`**, **`ag`**, **`ack`** in Bash/Shell — use `ministr_survey` instead
- ❌ **`find`**, **`fd`**, **`ls -R`** in Bash/Shell — use `ministr_toc` instead
- ❌ **`cat | grep`**, **`cmd | head`**, **`cmd | tail`**, **`cmd | wc`** — use ministr tools
- ❌ **ANY piped shell command** for code exploration — use ministr tools
- ❌ **Reading files** for exploration — use `ministr_symbols` → `ministr_definition`

### Allowed Uses of Shell/Bash

Shell is ONLY acceptable for: building code, running tests, installing dependencies, git operations, and running the project. NEVER for searching, file discovery, or piped exploration.

### Allowed Uses of file Read

File Read is ONLY acceptable immediately before Edit — never for exploration or discovery.

## Required Tool Mapping

| Instead of… | Use… |
|-------------|------|
| `grep` / `Grep` / text search | `ministr_survey(query: "...")` — semantic search across docs and code |
| `find` / `Glob` / file listing | `ministr_toc` — structural overview of the indexed corpus |
| Reading a file to find symbols | `ministr_symbols(query: "name")` — find by name/kind/module |
| Reading a file for a specific function | `ministr_definition(symbol_id: "...")` — get full source |
| Checking who calls a function | `ministr_references(symbol_id: "...")` — find all callers |

## Workflow

1. **Start with `ministr_survey`** for any question about the codebase
2. **Use `ministr_symbols`** to find specific code symbols
3. **Use `ministr_definition` or `ministr_read`** to get full source
4. **Use `ministr_references`** before modifying shared code (find all callers)
5. **Use `ministr_bridge`** before modifying cross-language boundaries (Tauri, FFI, etc.)
