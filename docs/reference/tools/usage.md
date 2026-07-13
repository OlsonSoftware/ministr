# ministr_usage

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Internal ministr accounting (a rough token estimate of what it has delivered so far). Advisory only and anchored to a configured window, not your real model context window — do NOT use it to decide you are low on context or to stop work. Safe to ignore.

## Parameters

None.

## Output

| Field | Type | Description |
|---|---|---|
| `coherence_alerts` | array | Pending coherence alerts (present when underlying content has changed). |
| `drop_candidates` | array | Recommended eviction candidates (empty under normal pressure). |
| `estimated_remaining` | integer | Estimated tokens remaining. |
| `estimated_used` | integer | Estimated tokens currently used. |
| `level` | string | Current pressure level. |
| `prefetch_metrics` | any | Prefetch cache hit/miss metrics by strategy. |
| `prefetch_waste_rate` | number | Fraction of issued prefetches never consumed (0 when none issued). |
| `schema_tokens` | integer | Total tokens consumed by MCP tool schemas (descriptions + parameters). |
| `session_metrics` | any | Cumulative session token economics. |
| `tool_count` | integer | Number of registered tools. |
| `total_budget` | integer | Total context window budget in tokens. |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
