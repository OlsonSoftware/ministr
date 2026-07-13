# ministr_references

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> All callers, implementors, and importers of a code symbol. Call before deleting or significantly modifying any non-trivial public symbol — zero references means safe to delete.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `col` | integer | no | Position byte column. |
| `cursor` | string | no | Cursor. |
| `file` | string | no | Position file. |
| `limit` | integer | no | Limit. |
| `line` | integer | no | Position line (1-based). |
| `offset` | integer | no | Offset. |
| `project` | string | no | Linked project. |
| `ref_kind` | string | no | Kind: calls, implements, imports, uses, bridge. |
| `symbol_id` | string | no | Symbol ID. |
| `through_implementors` | boolean | no | Include co-implementor callers. |

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
