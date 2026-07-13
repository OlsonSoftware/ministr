# ministr_dead

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Find symbols with zero references — candidates for safe deletion. Filters out `pub` symbols, entry points, and trivial helpers. Double-check with `ministr_references` before deleting since dynamic dispatch isn't tracked.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `cursor` | string | no | Cursor. |
| `kind` | string | no | Optional symbol kind filter (e.g. 'function', 'struct') |
| `limit` | integer | no | Maximum results to return. Default 50, capped at 500. |
| `min_lines` | integer | no | Skip symbols whose body is shorter than this many lines. Default 1. |
| `module` | string | no | Optional module path prefix filter |
| `offset` | integer | no | Offset. |
| `project` | string | no | Linked project. |

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
