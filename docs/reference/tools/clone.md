# ministr_clone

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Clone a git repository and index its content. Supports sparse checkout. Cached clones are reused.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `branch` | string | no | Optional branch to clone (defaults to repository default) |
| `paths` | array of string | no | Optional paths for sparse checkout (e.g. ['docs', 'src']). Omit for full checkout. |
| `repo` | string | no | Remote git repository URL to clone (e.g. 'https://github.com/owner/repo.git') |

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

Annotations: open-world.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
