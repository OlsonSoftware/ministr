---
description: Find all callers, implementors, and importers of a code symbol using ministr_references. Run this BEFORE modifying or deleting any shared symbol.
---

Find references to: $ARGUMENTS

If the input is a symbol ID (e.g. from a previous `ministr_symbols` result), pass it directly as `symbol_id`. Otherwise, resolve the name to a symbol via `ministr_symbols` first, then call `ministr_references` on the result.

Zero references means the symbol is safe to delete. Non-zero means every call site needs to be updated alongside any change to the symbol.
