# iris Playbook

Decision guide for using iris tools effectively in this project.

## Decision Tree

### "I need to understand something"

- **Vague question** → `iris_survey(query: "natural language question")`
- **Know the symbol name** → `iris_symbols(query: "name")` → `iris_definition(symbol_id)`
- **Know the file** → `iris_toc(document_id: "path")` → `iris_read(section_id)`
- **Need project layout** → `iris_toc(limit: 100)`

### "I need to change something"

- **Before touching shared code:**
  1. `iris_references(symbol_id)` — who calls it? who imports it?
  2. Only then `Read` → `Edit`

- **Before deleting code:**
  1. `iris_references(symbol_id)` — is anything still using it?
  2. Zero references = safe to delete

### "I need to find something"

- **A concept** → `iris_survey`
- **A specific symbol** → `iris_symbols`
- **All symbols of a kind** → `iris_symbols(kind: "struct")` or `iris_symbols(module: "name")`

## Anti-Patterns

- **Don't `Read` to explore.** Use `iris_read` or `iris_definition`.
- **Don't skip `iris_references` before modifying shared code.**
