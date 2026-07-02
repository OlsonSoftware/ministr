# ministr_symbols

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Find code symbols (functions, structs, traits, etc.) by name, kind, module, or visibility. Pair with ministr_definition for source and ministr_references before modifying.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `kind` | string | no | Exact kind filter: 'function', 'struct', 'trait', 'enum', 'impl', 'const', 'static', 'type', 'mod' |
| `limit` | integer | no | Maximum number of entries to return (default: 100) |
| `module` | string | no | Module path prefix filter (e.g. 'config' matches config::sub) |
| `offset` | integer | no | Number of entries to skip (default: 0) |
| `project` | string | no | Optional linked-project label. Omit for primary corpus. |
| `query` | string | no | Fuzzy symbol name search (case-insensitive substring match) |
| `visibility` | string | no | Exact visibility filter: 'pub', 'pub(crate)', 'pub(super)', '' |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
