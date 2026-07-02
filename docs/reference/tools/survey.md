# ministr_survey

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Search the indexed corpus by natural-language query. Start here for any vague question; follow up with ministr_read (full text) or ministr_extract (atomic claims) on top results.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `corpus_boost` | object | no | Optional per-corpus score multipliers for cross-corpus ranking. Map\<corpus_id, multiplier\>; absent corpora default to 1.0; clamped to [0, 10]. Use 2.0 to float your own repo above Atlas hits, 0.0 to suppress a corpus. |
| `corpus_ids` | array of string | no | Optional cross-corpus list. When set and non-empty, fans the query out across each corpus_id (own corpora or Atlas slugs), tags hits with source_corpus, and merges results by score. Omit to query a single corpus. |
| `project` | string | no | Optional linked-project label (from .ministr.toml [[linked]]). Omit for the session's primary corpus. Call ministr_projects to list labels. |
| `query` | string | no | Natural language query to search the corpus |
| `top_k` | integer | no | Maximum number of results to return |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
