# Agent Instructions

This project uses **iris** as an MCP server for semantic code search and navigation.
All AI agents working on this codebase **MUST** use iris tools instead of built-in alternatives.

## MCP Server: iris

iris is automatically configured via `.mcp.json` (Claude Code), `.vscode/mcp.json` (VS Code / Copilot), and `.cursor/mcp.json` (Cursor).

### Tool Reference

| Tool | Purpose |
|------|---------|
| `iris_survey(query)` | Semantic search across docs and code. **Start here.** |
| `iris_symbols(query)` | Find structs, functions, traits, enums by name/kind/module. |
| `iris_definition(symbol_id)` | Get full source of a symbol by ID. |
| `iris_references(symbol_id)` | Find callers, implementors, importers of a symbol. |
| `iris_read(section_id)` | Read a section by ID. |
| `iris_extract(section_id)` | Get atomic claims from a section. |
| `iris_toc` | Structural overview of the indexed corpus. |
| `iris_bridge(query)` | Cross-language bridge links (Tauri, PyO3, NAPI, etc.). |

### PROHIBITED — Do NOT Use for Exploration

**These are BLOCKED and must NEVER be used for code discovery or search:**

- ❌ `grep`, `rg`, `ripgrep`, `ag`, `ack`, `egrep`, `fgrep` → use `iris_survey`
- ❌ `find`, `fd`, `ls -R`, `tree`, directory listing → use `iris_toc`
- ❌ `cat file | grep`, `cmd | head`, `cmd | tail`, `cmd | wc` → use iris tools
- ❌ Built-in Grep/Glob tools → use `iris_survey` / `iris_toc`
- ❌ Reading files for exploration → use `iris_symbols` → `iris_definition`
- ❌ Any Shell/Bash/Terminal command for search or file discovery

**Allowed uses of Shell/Bash:** building, testing, git, installing dependencies, running the project.
**Allowed uses of file Read:** only immediately before Edit — never for exploration.

### Required Tool Mapping

| Instead of… | Use… |
|-------------|------|
| Grep / text search | `iris_survey` |
| Glob / file listing | `iris_toc` |
| Reading files for exploration | `iris_symbols` → `iris_definition` |
| Finding references manually | `iris_references` |

### Workflow

1. `iris_survey` → understand concepts, find relevant code
2. `iris_symbols` → locate specific symbols
3. `iris_definition` / `iris_read` → get full source
4. `iris_references` → check impact before modifying
5. `iris_bridge` → check cross-language boundaries
6. Only then: `Read` → `Edit`
