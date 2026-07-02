# Search

`ministr_survey` is neither a text grep nor a plain vector lookup. A query
runs through up to four stages:

## 1. Multi-resolution dense search

The query is embedded and searched against every vector resolution at once —
from document summaries down to individual claims and code symbols.
Candidates are scored by cosine similarity and max-pooled per content item:
of all the resolutions an item matched at, only its best-scoring vector
survives, so the top-k always holds distinct content. A question can be
answered by a doc paragraph, a single claim, or a function — whichever
actually matches.

## 2. Hybrid sparse fusion

With `sparse_weight > 0` (see
[configuration](../guides/configuration.md)), ingestion also builds a
sparse keyword index and dense + sparse rankings are fused with weighted
reciprocal-rank fusion. This is the exact-identifier recovery path for
code: a search naming a specific function ranks it first even when
embedding similarity alone would not. The default sparse encoder is
zero-model and structural — deterministic, no download; `sparse_encoder =
"splade"` opts into the neural alternative.

## 3. Matryoshka two-stage rescoring

If the corpus uses a truncated embedding `dimension`, coarse hits from the
small-dimension index are re-scored against stored full-dimension vectors —
small index, full-precision ranking.

## 4. Cross-encoder reranking (optional)

With a `reranker_model` configured, retrieval over-fetches candidates and a
cross-encoder rescores them against the query before truncation. Off by
default.

## Session awareness

Results the agent already received are excluded server-side (the response
reports `deduplicated_count`), so repeated searching never wastes context on
repeats. Strong hits come back with `next_actions` suggestions. See
[sessions](sessions.md).

## Cross-corpus search

`corpus_ids` fans a query out across multiple corpora
([linked projects](../guides/configuration.md) or cloned repos), tags each
hit with `source_corpus`, and merges by score; `corpus_boost` applies
per-corpus multipliers.
