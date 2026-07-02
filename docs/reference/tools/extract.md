# ministr_extract

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Atomic claims from a section, optionally query-filtered. Cheaper than ministr_read when you don't need full prose.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `project` | string | no | Optional linked-project label. Omit for primary corpus. |
| `query` | string | no | Optional query to filter claims by relevance |
| `section_id` | string | no | Section ID to extract claims from |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
