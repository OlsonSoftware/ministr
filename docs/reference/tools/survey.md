# ministr_survey

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Search the indexed corpus by natural-language query. Start here for any vague question; follow up with ministr_read (full text) or ministr_extract (atomic claims) on top results.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `corpus_boost` | object | no | Corpus score multipliers (0..10). |
| `corpus_ids` | array of string | no | Corpus IDs/Atlas slugs; overrides project. |
| `cursor` | string | no | Cursor. |
| `limit` | integer | no | Limit (cap 100). |
| `offset` | integer | no | Offset. |
| `project` | string | no | Linked project. |
| `query` | string | no | Natural-language search query. |
| `top_k` | integer | no | Legacy limit (default 10). |

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
