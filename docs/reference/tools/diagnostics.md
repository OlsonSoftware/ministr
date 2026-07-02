# ministr_diagnostics

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Run the project's own toolchain(s) (cargo/tsc/eslint/ruff/go vet/…, plus any SARIF-emitting tool) and return bounded STRUCTURED diagnostics (file, range, severity, code, message), errors first, each cross-linked to the enclosing symbol. The agentic verify step — structured compiler/lint feedback as data, never raw build logs. Language-agnostic.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `languages` | array of string | no | Restrict to these toolchain languages (e.g. 'rust','typescript','python','go'). Omit to run every detected toolchain. |
| `limit` | integer | no | Maximum diagnostics to return. Default 100, capped at 500. |
| `project` | string | no | Optional linked-project label. Omit for primary corpus. |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
