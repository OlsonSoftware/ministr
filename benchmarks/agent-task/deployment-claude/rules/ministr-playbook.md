# ministr Playbook

Decision guide for using ministr tools effectively in this project.

## Decision Tree

### "I need to understand something"

- **Vague question** → `ministr_survey(query: "natural language question")`
- **Know the symbol name** → `ministr_symbols(query: "name")` → `ministr_definition(symbol_id)`
- **Know the file** → `ministr_toc(document_id: "path")` → `ministr_read(section_id)`
- **Need project layout** → `ministr_toc(limit: 100)`

### "I need to change something"

- **Before touching shared code:**
  1. `ministr_references(symbol_id)` — who calls it? who imports it?
  2. Only then `Read` → `Edit`

- **Before deleting code:**
  1. `ministr_references(symbol_id)` — is anything still using it?
  2. Zero references = safe to delete

### "I need to find something"

- **A concept** → `ministr_survey`
- **A specific symbol** → `ministr_symbols`
- **All symbols of a kind** → `ministr_symbols(kind: "struct")` or `ministr_symbols(module: "name")`

## Anti-Patterns

- **Don't `Read` to explore.** Use `ministr_read` or `ministr_definition`.
- **Don't skip `ministr_references` before modifying shared code.**
