# ministr_definition

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Full source of a code symbol by ID. Call ministr_references first if you intend to modify or delete the symbol. Pass blame=true for git authorship of the symbol's lines.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `blame` | boolean | no | Include git blame. |
| `col` | integer | no | Position byte column. |
| `context_lines` | integer | no | Context lines. |
| `file` | string | no | Position file. |
| `include_body` | boolean | no | Include body. |
| `line` | integer | no | Position line (1-based). |
| `max_lines` | integer | no | Line limit (cap 1000). |
| `outline_only` | boolean | no | Outline only. |
| `project` | string | no | Linked project. |
| `source_corpus` | string | no | Source corpus fallback. |
| `start_byte` | integer | no | Continuation byte. |
| `start_line` | integer | no | Continuation line. |
| `symbol_id` | string | no | Symbol ID. |

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
