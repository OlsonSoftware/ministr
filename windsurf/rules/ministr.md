# ministr MCP — Codebase Navigation

This project uses ministr as an MCP server for semantic code search.
ministr is the **preferred** tool for code *exploration*. It does not
restrict normal shell work.

## Policy

- Prefer ministr MCP tools for code discovery, search, and navigation.
- The built-in `Grep` / `Glob` tools should not be used for exploration —
  `ministr_survey` / `ministr_toc` are the equivalents.
- The shell is unrestricted: pipelines, `git`, dependency installs,
  builds/tests, and filtering command output (`cargo test | grep`,
  `… | tail`) all run normally. A *leading* `grep`/`find` is a hint to
  prefer ministr — it is fine when filtering output or doing a
  filesystem operation.
- Read files only immediately before editing them.

## Tool Mapping

| For… | Prefer… |
|------|---------|
| code/text search | `ministr_survey(query: "...")` |
| file/structure discovery | `ministr_toc` |
| a symbol's source | `ministr_symbols` → `ministr_definition` |
| who calls a symbol | `ministr_references(symbol_id: "...")` |

## Workflow

1. `ministr_survey` → find relevant code
2. `ministr_symbols` → locate specific symbols
3. `ministr_definition` / `ministr_read` → get full source
4. `ministr_references` → check impact before modifying
5. Only then: Read → Edit
