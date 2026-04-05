# iris MCP — Codebase Navigation (MANDATORY)

This project uses iris as an MCP server for semantic code search.

## CRITICAL: Tool Restrictions

**You MUST use iris MCP tools for ALL codebase exploration.**

### Prohibited Tools and Patterns

- ❌ `grep`, `rg`, `ag`, `ack` in terminal — use `iris_survey` instead
- ❌ `find`, `fd`, `ls -R` in terminal — use `iris_toc` instead
- ❌ `cat | grep`, piped shell commands — use iris tools
- ❌ Reading files for exploration — use `iris_symbols` → `iris_definition`

### Allowed Uses of Shell

Shell is ONLY acceptable for: building code, running tests, installing dependencies, git operations, and running the project. NEVER for searching, file discovery, or piped exploration.

### Required Tool Mapping

| Instead of… | Use… |
|-------------|------|
| Grep / text search | `iris_survey(query: "...")` |
| Find / file listing | `iris_toc` |
| Reading files for exploration | `iris_symbols` → `iris_definition` |
| Finding references | `iris_references(symbol_id: "...")` |

### Workflow

1. `iris_survey` → find relevant code
2. `iris_symbols` → locate specific symbols
3. `iris_definition` / `iris_read` → get full source
4. `iris_references` → check impact before modifying
5. Only then: Read → Edit
