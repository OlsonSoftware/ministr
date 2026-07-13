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

## Output

| Field | Type | Description |
|---|---|---|
| `chunk` | string | The log span (delta mode) or matched lines joined (query mode). |
| `matched_lines` | integer | Matched line count (query mode only). |
| `next_offset` | integer | Cursor for the next page (delta mode only). |
| `remaining_bytes` | integer | Bytes not yet delivered after this page (delta mode only). |
| `run_id` | string | Run id. |
| `status` | string | Lifecycle state: `running` \| `exited` \| `killed` \| `timed_out`. |

Annotations: read-only.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
