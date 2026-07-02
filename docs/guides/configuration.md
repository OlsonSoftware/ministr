# Configuration

Two files. Project settings live in `.ministr.toml` at your repo root
(discovered by walking up from the working directory); machine-wide defaults
live in `~/.ministr/config.toml` (optional — a missing file means all
defaults). For model settings the project file wins over the global default.

## Project: `.ministr.toml`

`ministr init` writes a starting point. Minimal config:

```toml
[corpus]
paths = ["src", "docs"]
```

### `[corpus]`

| Key | Type | Default | Effect |
|---|---|---|---|
| `paths` | list | `["."]` | repo-relative paths to index |
| `ignore` | list of globs | `[]` | extra ignore patterns, on top of gitignore and built-ins |
| `model` | string | — | embedding model for this project (overrides the global default) |
| `dimension` | integer | — | Matryoshka truncation target; only useful with Matryoshka-capable models such as `nomic-embed-text-v1.5` |
| `rerank_depth` | integer | `100` | coarse candidates rescored at full dimension when `dimension` is set; `0` disables |
| `sparse_weight` | float | — (dense-only) | hybrid retrieval: the sparse share of the dense+sparse fusion, `0.0`–`1.0` |
| `sparse_encoder` | string | `"ast"` | sparse encoder for hybrid retrieval: `"ast"` (zero-model, deterministic) or `"splade"` (neural, downloads a model) |

### Hybrid retrieval (`sparse_weight`)

With `sparse_weight > 0`, ingestion also builds a sparse keyword index and
every search fuses dense and sparse rankings — the exact-identifier recovery
path for code, where a query naming a specific function ranks it first even
when embedding similarity alone would not.

Pick the weight by corpus type; there is no good global value. On ministr's
deterministic evaluation corpora (2026-06), code retrieval peaked at `0.6`
and declined again above it, while documentation/prose precision regressed
at every tested weight. `ministr init` writes `0.6` for code projects and
leaves prose projects dense-only.

The default encoder is structural, not neural: a BM25F-style scorer over the
roles ministr derives from your code (definition name, doc comment,
signature, body). It needs no model download and is fully deterministic. Set
`sparse_encoder = "splade"` for the neural encoder, which downloads a model
and keeps a small edge on human-phrased queries at the cost of slower
ingest. Switching encoders discards the sparse index; re-index to repopulate
it.

### Extra sources

```toml
[[corpus.include]]   # merge an external local directory into this corpus
path = "~/other/shared-lib"

[[corpus.git]]       # clone and index a git repository
repo = "https://github.com/org/dep"

[[corpus.cloud]]     # fetch a pre-built index bundle instead of indexing
url = "https://example.com/dep.ministr-index"
```

### `[[linked]]` — multi-project querying

```toml
[[linked]]
path = "~/Code/other-repo"
label = "other"
```

A linked project is not merged — it keeps its own index and identity. Agents
target it with the `project: "other"` argument on any query tool
(`ministr_projects` lists labels).

### `[agent]`

```toml
[agent]
rules = ["Always run just validate before committing"]
```

Rules appended to all generated agent advisory files (`.claude/rules/`,
`.cursor/rules/`, `.github/copilot-instructions.md`, and the rest).

## Global: `~/.ministr/config.toml`

| Key | Type | Default | Effect |
|---|---|---|---|
| `data_dir` | path | `~/.ministr` | where indexes, models, and logs live |
| `default_model` | string | `all-MiniLM-L6-v2` | embedding model for new projects |
| `default_context_budget` | integer | `100000` | advisory token budget for new sessions |
| `log_format` | string | `pretty` | `pretty` or `json` |
| `corpus_paths` | list | `[]` | fallback corpus sources: local paths, `https://` URLs, or `github://owner/repo` |
| `reranker_model` | string | unset (off) | cross-encoder reranking of query results |

## Choosing an embedding model

The default, `all-MiniLM-L6-v2` (384d), is small and fast. Measured guidance
from ministr's deterministic evaluation corpora (2026-06):

- `embedding-gemma-300m` (768d) won on code — recall and ranking both
  improved over the default — but it is roughly 14× the parameters, so
  ingestion is correspondingly slower. A real tradeoff, not a free upgrade.
- The code-branded `jina-embeddings-v2-base-code` scored *worse* than the
  generic default on ministr's own ground-truth code queries. "Code model"
  labels don't automatically transfer — evaluate on your corpus before
  switching.
- For gemma, run the native 768 dimensions. If index memory matters,
  `dimension = 256` with the default `rerank_depth = 100` recovers most of
  the quality with a 3× smaller index; truncating *without* the rerank costs
  real code-retrieval quality.

Changing a project's model requires a re-index.
