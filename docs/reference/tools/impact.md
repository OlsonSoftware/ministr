# ministr_impact

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Transitive blast radius of changing a symbol. Returns every caller / implementor / importer N levels deep, plus distinct files, distinct test files, and a low/medium/high risk score. Use BEFORE recommending a non-trivial refactor.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `direction` | string | no | Call-graph direction: 'incoming' = transitive callers / blast radius (default), 'outgoing' = transitive callees (what this symbol calls). |
| `max_depth` | integer | no | Maximum BFS depth to walk the call graph. Default 3, capped at 10. |
| `project` | string | no | Optional linked-project label. Omit for primary corpus. |
| `range` | string | no | Git revision range (e.g. 'main..HEAD', 'HEAD~3'). When set, analyzes the diff-aware blast radius: which indexed symbols the range touched and the union of what they can break. Overrides symbol_id. |
| `repo_path` | string | no | Path inside the git work tree to resolve `range` against. Defaults to the server's working directory. Only used with `range`. |
| `symbol_id` | string | no | Symbol ID whose blast radius should be analyzed (from ministr_symbols results) |
| `tests_only` | boolean | no | When true, keep only nodes in test files. With direction 'incoming' this answers 'which tests transitively exercise this symbol' (the minimal test set for a change). |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
