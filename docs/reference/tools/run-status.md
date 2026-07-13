# ministr_run_status

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Poll a run's status (running/exited/killed/timed_out, exit code, duration, bytes).

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `run_id` | string | yes | Run id from ministr_run |

## Output

| Field | Type | Description |
|---|---|---|
| `bytes_total` | integer | Exact bytes produced so far (final after exit). |
| `duration_ms` | integer | Wall-clock duration in milliseconds. |
| `exit_code` | integer | Exit code (None while running or signal-killed). |
| `run_id` | string | Run id. |
| `status` | string | Lifecycle state: `running` \| `exited` \| `killed` \| `timed_out`. |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
