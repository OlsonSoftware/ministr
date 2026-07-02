# ministr_run_logs

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Page a run's captured log (delta: only what you haven't seen) or filter it with query=substring.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `from_offset` | integer | no | Explicit byte offset (overrides the session cursor) |
| `max_bytes` | integer | no | Max bytes per page (default 16384) |
| `query` | string | no | Substring filter: return matching lines instead of paging |
| `run_id` | string | yes | Run id from ministr_run |

Annotations: read-only.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
