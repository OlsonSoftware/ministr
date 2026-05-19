---
description: Find symbols with zero references — candidates for safe deletion. Uses ministr_dead.
---

Find dead code matching: $ARGUMENTS

Use the `ministr_dead` MCP tool. Optional filters in `$ARGUMENTS`: `kind:fn`, `kind:struct`, `module:foo`, `min_lines:5` (skip trivial helpers). With no filters, returns all non-public symbols with zero references, excluding `main`, `#[test]` items, and other entry-point heuristics.

Before deleting any reported symbol, double-check with `ministr_references` — `ministr_dead` uses the static reference graph and won't catch indirect uses through trait objects or dynamic dispatch.
