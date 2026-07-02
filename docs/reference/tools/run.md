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

Annotations: destructive · open-world.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
