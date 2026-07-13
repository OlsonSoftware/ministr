# ministr_toc

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Structural overview (table of contents) of the indexed corpus. Use to orient on an unfamiliar codebase before drilling in.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `cursor` | string | no | Cursor. |
| `document_id` | string | no | Optional document ID to filter the table of contents to a single document |
| `limit` | integer | no | Maximum number of entries to return (default: 100) |
| `offset` | integer | no | Number of entries to skip (default: 0) |
| `project` | string | no | Linked project. |
| `source_corpus` | string | no | Source corpus fallback. |

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
