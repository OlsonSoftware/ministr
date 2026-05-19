---
description: Semantic search across this codebase using ministr_survey. Returns the relevant slice (a section, claim, or symbol) for a natural-language question.
---

Search the indexed corpus for: $ARGUMENTS

Use the `ministr_survey` MCP tool with the query above. Inspect the response, then follow up with `ministr_definition` (for code symbols), `ministr_read` (for prose sections), or `ministr_extract` (for atomic claims) on the top results as needed.

Do not shell out to `grep` or read files directly — `ministr_survey` already returns the precise slice.
