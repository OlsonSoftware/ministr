# ministr_impact

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Transitive blast radius of changing a symbol. Returns every caller / implementor / importer N levels deep, plus distinct files, distinct test files, and a low/medium/high risk score. Use BEFORE recommending a non-trivial refactor.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `cursor` | string | no | Cursor. |
| `direction` | string | no | Direction: incoming or outgoing. |
| `limit` | integer | no | Limit (cap 500). |
| `max_depth` | integer | no | Depth (cap 10). |
| `offset` | integer | no | Offset. |
| `project` | string | no | Linked project. |
| `range` | string | no | Git range; overrides symbol_id. |
| `repo_path` | string | no | Git work tree. |
| `symbol_id` | string | no | Symbol ID. |
| `tests_only` | boolean | no | Tests only. |

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
