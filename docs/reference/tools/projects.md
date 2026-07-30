# ministr_projects

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> List every project route you can query in this session: the current project plus any linked ones. Pass a returned `label` as the `project` argument to other tools, or omit `project` for the current one. Linked projects come from `.ministr.toml` `[[linked]]` entries (or `ministr_clone` results).

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `cursor` | string | no | Continuation cursor from a prior page. |
| `limit` | integer | no | Maximum projects. Default 100, capped at 500. |
| `offset` | integer | no | Projects to skip. Default 0. |

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
