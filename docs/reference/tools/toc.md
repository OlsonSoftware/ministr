# ministr_toc

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Structural overview (table of contents) of the indexed corpus. Use to orient on an unfamiliar codebase before drilling in.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `document_id` | string | no | Optional document ID to filter the table of contents to a single document |
| `limit` | integer | no | Maximum number of entries to return (default: 100) |
| `offset` | integer | no | Number of entries to skip (default: 0) |
| `project` | string | no | Optional linked-project label. Omit for primary corpus. |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
