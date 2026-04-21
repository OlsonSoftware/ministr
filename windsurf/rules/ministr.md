# ministr MCP — Codebase Navigation (MANDATORY)

This project uses ministr as an MCP server for semantic code search.

## CRITICAL: Tool Restrictions

**You MUST use ministr MCP tools for ALL codebase exploration.**

### Prohibited Tools and Patterns

- ❌ `grep`, `rg`, `ag`, `ack` in terminal — use `ministr_survey` instead
- ❌ `find`, `fd`, `ls -R` in terminal — use `ministr_toc` instead
- ❌ `cat | grep`, piped shell commands — use ministr tools
- ❌ Reading files for exploration — use `ministr_symbols` → `ministr_definition`

### Allowed Uses of Shell

Shell is ONLY acceptable for: building code, running tests, installing dependencies, git operations, and running the project. NEVER for searching, file discovery, or piped exploration.

### Required Tool Mapping

| Instead of… | Use… |
|-------------|------|
| Grep / text search | `ministr_survey(query: "...")` |
| Find / file listing | `ministr_toc` |
| Reading files for exploration | `ministr_symbols` → `ministr_definition` |
| Finding references | `ministr_references(symbol_id: "...")` |

### Workflow

1. `ministr_survey` → find relevant code
2. `ministr_symbols` → locate specific symbols
3. `ministr_definition` / `ministr_read` → get full source
4. `ministr_references` → check impact before modifying
5. Only then: Read → Edit
