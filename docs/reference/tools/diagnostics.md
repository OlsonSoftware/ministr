# ministr_diagnostics

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Run the project's own toolchain(s) (cargo/tsc/eslint/ruff/go vet/…, plus any SARIF-emitting tool) and return bounded STRUCTURED diagnostics (file, range, severity, code, message), errors first, each cross-linked to the enclosing symbol. The agentic verify step — structured compiler/lint feedback as data, never raw build logs. Language-agnostic.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `cursor` | string | no | Cursor. |
| `languages` | array of string | no | Restrict to these toolchain languages (e.g. 'rust','typescript','python','go'). Omit to run every detected toolchain. |
| `limit` | integer | no | Maximum diagnostics to return. Default 100, capped at 500. |
| `offset` | integer | no | Number of diagnostics to skip. Default 0. |
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
