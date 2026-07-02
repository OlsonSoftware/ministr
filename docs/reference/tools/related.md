# ministr_related

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Follow relationship edges (references, contradicts, depends_on, updates) from a claim. Use when one claim's truth depends on another.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `claim_id` | string | no | Claim ID to find related claims for |
| `project` | string | no | Optional linked-project label. Omit for primary corpus. |
| `relation_types` | array of string | no | Optional filter: 'references', 'contradicts', 'depends_on', 'updates' |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
