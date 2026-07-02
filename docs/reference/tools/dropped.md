# ministr_dropped

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Call immediately after dropping content you previously received. Keeps ministr's view of what you still have accurate; without this, future ministr_read calls on dropped IDs return short 'already delivered' stubs instead of the full text.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `content_ids` | array of string | no | Content IDs the agent has dropped from its context |

Annotations: idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
