# ministr_clone

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Clone a git repository and index its content. Supports sparse checkout. Cached clones are reused.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `branch` | string | no | Optional branch to clone (defaults to repository default) |
| `paths` | array of string | no | Optional paths for sparse checkout (e.g. ['docs', 'src']). Omit for full checkout. |
| `repo` | string | no | Remote git repository URL to clone (e.g. 'https://github.com/owner/repo.git') |

Annotations: open-world.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
