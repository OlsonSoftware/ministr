# Agent Instructions

This project uses **ministr** as an MCP server for semantic code search and navigation.
All AI agents working on this codebase **MUST** use ministr tools instead of built-in alternatives.

## MCP Server: ministr

ministr is automatically configured via `.mcp.json` (Claude Code), `.vscode/mcp.json` (VS Code / Copilot), and `.cursor/mcp.json` (Cursor).

### Tool Reference

| Tool | Purpose |
|------|---------|
| `ministr_survey(query)` | Semantic search across docs and code. **Start here.** |
| `ministr_symbols(query)` | Find structs, functions, traits, enums by name/kind/module. |
| `ministr_definition(symbol_id)` | Get full source of a symbol by ID. |
| `ministr_references(symbol_id)` | Find callers, implementors, importers of a symbol. |
| `ministr_read(section_id)` | Read a section by ID. |
| `ministr_extract(section_id)` | Get atomic claims from a section. |
| `ministr_toc` | Structural overview of the indexed corpus. |
| `ministr_bridge(query)` | Cross-language bridge links (Tauri, PyO3, NAPI, etc.). |

### PROHIBITED — Do NOT Use for Exploration

**These are BLOCKED and must NEVER be used for code discovery or search:**

- ❌ `grep`, `rg`, `ripgrep`, `ag`, `ack`, `egrep`, `fgrep` → use `ministr_survey`
- ❌ `find`, `fd`, `ls -R`, `tree`, directory listing → use `ministr_toc`
- ❌ `cat file | grep`, `cmd | head`, `cmd | tail`, `cmd | wc` → use ministr tools
- ❌ Built-in Grep/Glob tools → use `ministr_survey` / `ministr_toc`
- ❌ Reading files for exploration → use `ministr_symbols` → `ministr_definition`
- ❌ Any Shell/Bash/Terminal command for search or file discovery

**Allowed uses of Shell/Bash:** building, testing, git, installing dependencies, running the project.
**Allowed uses of file Read:** only immediately before Edit — never for exploration.

### Required Tool Mapping

| Instead of… | Use… |
|-------------|------|
| Grep / text search | `ministr_survey` |
| Glob / file listing | `ministr_toc` |
| Reading files for exploration | `ministr_symbols` → `ministr_definition` |
| Finding references manually | `ministr_references` |

### Workflow

1. `ministr_survey` → understand concepts, find relevant code
2. `ministr_symbols` → locate specific symbols
3. `ministr_definition` / `ministr_read` → get full source
4. `ministr_references` → check impact before modifying
5. `ministr_bridge` → check cross-language boundaries
6. Only then: `Read` → `Edit`
