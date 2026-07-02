# ministr_dead

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Find symbols with zero references — candidates for safe deletion. Filters out `pub` symbols, entry points, and trivial helpers. Double-check with `ministr_references` before deleting since dynamic dispatch isn't tracked.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `kind` | string | no | Optional symbol kind filter (e.g. 'function', 'struct') |
| `limit` | integer | no | Maximum results to return. Default 50, capped at 500. |
| `min_lines` | integer | no | Skip symbols whose body is shorter than this many lines. Default 1. |
| `module` | string | no | Optional module path prefix filter |
| `project` | string | no | Optional linked-project label. Omit for primary corpus. |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
