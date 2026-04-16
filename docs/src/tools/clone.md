<div class="iris-tool-head">
<svg class="icon icon-xl iris-tool-icon"><use href="../assets/icons.svg#git-branch"/></svg>
</div>

# iris_clone

Clone a git repository and index it into the corpus.

## Parameters

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `url` | string | yes | — | Git URL (HTTPS or `github:owner/repo` shorthand) |
| `ref` | string | no | default branch | Branch, tag, or commit SHA to check out |
| `sparse_paths` | list of strings | no | — | Sparse-checkout paths (only index these directories) |

## Response

```json
{
  "clone_id": "github.com/serde-rs/serde",
  "ref": "v1.0.200",
  "commit_sha": "abc123...",
  "file_count": 87,
  "section_count": 1204,
  "symbol_count": 892,
  "budget_status": { ... }
}
```

## Behavior

- Clones into the iris data directory (`~/.iris/clones/<hash>`)
- Supports GitHub shorthand: `github:rust-lang/rust` → `https://github.com/rust-lang/rust.git`
- Sparse checkout dramatically reduces disk and index time for large monorepos
- Subsequent calls with the same URL are idempotent — the cache is reused if the ref hasn't changed
- Use `iris_refresh` to pull updates after upstream changes
- Cloned content is treated as a separate corpus root alongside local paths
