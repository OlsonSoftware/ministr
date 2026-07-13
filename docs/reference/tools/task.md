# ministr_task

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Poll a background task status. Deprecated: prefer MCP tasks/get.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `task_id` | string | no | Task ID to check status for |

## Output

| Field | Type | Description |
|---|---|---|
| `createdAt` | string | ISO-8601 creation timestamp. |
| `lastUpdatedAt` | string | ISO-8601 timestamp for the most recent status change. |
| `pollInterval` | integer | Suggested polling interval in milliseconds. |
| `status` | string | Current status: `working`, `completed`, `failed`, `cancelled`, or `input_required`. |
| `statusMessage` | string | Human-readable status message. |
| `taskId` | string | Unique task identifier. |
| `ttl` | integer | Retention window in milliseconds. |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
