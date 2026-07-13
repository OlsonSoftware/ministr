# ministr_inspect

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Bounded definition, callers, callees, implementations, tests, and bridges for one symbol. Use after survey/symbols when you need impact context in one round trip; use granular tools to page one group.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `col` | integer | no |  |
| `file` | string | no |  |
| `include` | array | no | Groups: definition, callers, callees, implementors, imports, tests, bridges; empty means all. |
| `line` | integer | no |  |
| `max_per_group` | integer | no | Per-group limit (cap 50). |
| `max_source_lines` | integer | no | Source-line limit (cap 1000). |
| `project` | string | no | Linked project. |
| `symbol_id` | string | no | Symbol ID, or use file+line+col. |

## Output

| Field | Type | Description |
|---|---|---|
| `coherence_alerts` | array | Pending coherence alerts (present when underlying content has changed). |
| `completeness` | any | Whether absence is conclusive for the index generation queried. |
| `corpora` | array | Per-corpus status for fan-out/routed operations. |
| `error` | … | Stable error detail for partial/error responses. |
| `indexing_in_progress` | boolean | True when background corpus ingestion is still running. |
| `indexing_message` | string | Human-readable ingestion status message (e.g. "Checking 12/42 files"). |
| `next_actions` | array | Concrete next-tool-call suggestions, in priority order.  Coherence-driven (re-read changed sections) plus any per-handler hints (e.g. survey's top-result follow-up). Budget pressure no longer contributes entries here — see the struct-level note. |
| `result` | … | The tool-specific result data (varying — placed last for prefix stability). |
| `status` | any | Machine-readable operation status; empty successful results remain `ok`. |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
