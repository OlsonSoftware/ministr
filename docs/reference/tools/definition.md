# ministr_definition

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Full source of a code symbol by ID. Call ministr_references first if you intend to modify or delete the symbol. Pass blame=true for git authorship of the symbol's lines.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `blame` | boolean | no | When true, attach git blame metadata (per-author line counts + last commit) for the symbol's line range. Omitted when not a git repo. |
| `col` | integer | no | 0-based byte column for position-addressed lookup (requires file+line) |
| `file` | string | no | Position-addressed alternative to symbol_id: file path (with line+col) to resolve the symbol under the cursor |
| `line` | integer | no | 1-based line for position-addressed lookup (requires file+col) |
| `project` | string | no | Optional linked-project label. Omit for primary corpus. |
| `symbol_id` | string | no | Symbol ID to get the definition for (from ministr_symbols results) |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
