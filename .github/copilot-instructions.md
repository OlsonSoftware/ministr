# iris MCP — Codebase Navigation (MANDATORY)

This project uses [iris](https://github.com/AlrikOlson/iris-rs) as an MCP server for semantic code search.

## CRITICAL: Tool Restrictions

**You MUST use iris MCP tools for ALL codebase exploration.** The following are PROHIBITED:

### Prohibited Tools and Patterns

- ❌ **Grep tool** — use `iris_survey(query: "...")` instead
- ❌ **Glob tool** — use `iris_toc` instead
- ❌ **`grep`**, **`rg`**, **`ag`**, **`ack`** in Bash/Shell — use `iris_survey` instead
- ❌ **`find`**, **`fd`**, **`ls -R`** in Bash/Shell — use `iris_toc` instead
- ❌ **`cat | grep`**, **`cmd | head`**, **`cmd | tail`**, **`cmd | wc`** — use iris tools
- ❌ **ANY piped shell command** for code exploration — use iris tools
- ❌ **Reading files** for exploration — use `iris_symbols` → `iris_definition`

### Allowed Uses of Shell/Bash

Shell is ONLY acceptable for: building code, running tests, installing dependencies, git operations, and running the project. NEVER for searching, file discovery, or piped exploration.

### Allowed Uses of file Read

File Read is ONLY acceptable immediately before Edit — never for exploration or discovery.

## Required Tool Mapping

| Instead of… | Use… |
|-------------|------|
| `grep` / `Grep` / text search | `iris_survey(query: "...")` — semantic search across docs and code |
| `find` / `Glob` / file listing | `iris_toc` — structural overview of the indexed corpus |
| Reading a file to find symbols | `iris_symbols(query: "name")` — find by name/kind/module |
| Reading a file for a specific function | `iris_definition(symbol_id: "...")` — get full source |
| Checking who calls a function | `iris_references(symbol_id: "...")` — find all callers |

## Workflow

1. **Start with `iris_survey`** for any question about the codebase
2. **Use `iris_symbols`** to find specific code symbols
3. **Use `iris_definition` or `iris_read`** to get full source
4. **Use `iris_references`** before modifying shared code (find all callers)
5. **Use `iris_bridge`** before modifying cross-language boundaries (Tauri, FFI, etc.)
