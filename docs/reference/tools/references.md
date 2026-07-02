# ministr_references

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> All callers, implementors, and importers of a code symbol. Call before deleting or significantly modifying any non-trivial public symbol — zero references means safe to delete.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `col` | integer | no | 0-based byte column for position-addressed lookup (requires file+line) |
| `file` | string | no | Position-addressed alternative to symbol_id: file path (with line+col) to resolve the symbol under the cursor |
| `limit` | integer | no | Maximum number of entries to return (default: 100) |
| `line` | integer | no | 1-based line for position-addressed lookup (requires file+col) |
| `offset` | integer | no | Number of entries to skip (default: 0) |
| `project` | string | no | Optional linked-project label. Omit for primary corpus. |
| `ref_kind` | string | no | Optional reference kind filter: 'calls', 'implements', 'imports', 'uses', 'bridge' |
| `symbol_id` | string | no | Symbol ID to find references for (from ministr_symbols results) |
| `through_implementors` | boolean | no | When true, also include callers of the same-named method on co-implementor types (LSP references-including-overrides / type hierarchy). Bounded; combine with ref_kind 'calls' or omit ref_kind. |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
