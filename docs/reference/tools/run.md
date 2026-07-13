# ministr_run

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Run a shell command (recorded + captured). Returns exit code + a token-lean digest with every error line; full log via ministr_run_logs. background:true returns a run_id immediately.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `background` | boolean | no | Run in background; poll with ministr_run_status |
| `command` | string | yes | Shell command line to execute |
| `cwd` | string | no | Working directory; defaults to the first corpus root |
| `timeout_secs` | integer | no | Timeout seconds (default 600, max 3600) |

## Output

| Field | Type | Description |
|---|---|---|
| `bytes_total` | integer | Exact bytes the command produced. |
| `capture_truncated` | boolean | True when the engine's capture guard dropped middle output. |
| `digest` | … | Token-lean digest (None for background starts). |
| `duration_ms` | integer | Wall-clock duration in milliseconds. |
| `exit_code` | integer | Exit code (None while running or signal-killed). |
| `run_id` | string | Run id (use with `ministr_run_logs` / `ministr_run_status`). |
| `status` | string | Lifecycle state: `running` \| `exited` \| `killed` \| `timed_out`. |

Annotations: destructive · open-world.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
