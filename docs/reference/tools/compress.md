# ministr_compress

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Extractive TF-IDF summaries (roughly 60-80% shorter) for sections you want to keep referenceable without their full text. Pair with ministr_dropped after dropping the originals.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `content_ids` | array of string | no | Content IDs (section IDs) to generate compressed summaries for |
| `cursor` | string | no | Cursor. |
| `identities` | array | no | Exact corpus/content/resolution identities to compress. |
| `limit` | integer | no | Limit (cap 500). |
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

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
