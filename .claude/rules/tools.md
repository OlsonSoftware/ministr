# Tool Guide

## Codebase Navigation (iris)

| Tool | Purpose |
|------|---------|
| `iris_survey` | Primary discovery — semantic search across docs and code. Start here. |
| `iris_symbols` | Search code symbols by name, kind, module, or visibility. |
| `iris_definition` | Get full source definition of a symbol by ID. |
| `iris_references` | Find all references to a symbol (callers, implementors, importers). |
| `iris_read` | Read full section text by ID. Deduplicates and returns deltas. |
| `iris_extract` | Extract atomic claims from a section, optionally filtered by query. |
| `iris_related` | Follow dependency chains between claims. |
| `iris_toc` | Structural overview of the indexed corpus. |
| `iris_budget` | Check context budget status and eviction recommendations. |
| `iris_compress` | Generate compressed summaries for content you want to evict. |

Recommended workflow: `iris_survey` → `iris_symbols` → `iris_definition` → dig deeper with `iris_extract` / `iris_related`.

## Quality & Workflow (magistr)

| Tool | Purpose |
|------|---------|
| `magistr_gate` | Run quality gates (`all` / `check` / `test` / `lint` + custom). |
| `magistr_format` | Run the project formatter. |
| `magistr_set_phase` | Announce workflow phase transitions. |
| `magistr_roadmap_status` | Roadmap overview with progress. |
| `magistr_roadmap_tasks` | List tasks with filtering. |
| `magistr_roadmap_check` | Check off / uncheck tasks. |
| `magistr_roadmap_add_task` | Add new tasks. |
| `magistr_roadmap_remove_task` | Remove tasks. |
| `magistr_roadmap_sync` | Regenerate `ROADMAP.md` from canonical data. |
| `magistr_delete` | Delete files/directories (use instead of `rm`). |
| `magistr_list` | List directory contents. |

## Tool Preferences

- Use `iris_survey` instead of Glob/find/Grep for discovering code and docs.
- Use `iris_symbols` + `iris_definition` instead of Read for exploring code structure.
- Use `iris_extract` to get specific claims from a section instead of reading the whole thing.
- Use `Read` only immediately before `Edit` — for everything else, use iris tools.
- Use `magistr_gate` instead of running gate commands manually.
- Use `magistr_roadmap_*` tools instead of manually editing `ROADMAP.md`.
- Use `magistr_delete` instead of `rm` in Bash.
- Do not spawn sub-agents — work directly in the current session.
