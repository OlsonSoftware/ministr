# Search

`ministr_survey` is neither a text grep nor a plain vector lookup. A query
runs through up to four stages:

Survey is a discovery operation. Section hits contain a query-centered,
bounded excerpt (or a stored summary when it represents the match better),
not an implicit copy of the whole section. Every clipped result says that it
is truncated and reports its original and returned size. Follow its stable
locator with `ministr_read` when the full section is required. Symbol stubs
keep the signature, kind, file, and documentation needed to decide whether a
definition is worth opening.

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

Results the agent already received at the same corpus and resolution are
excluded server-side; `deduplicated_count` reports what was skipped. Strong
hits come back with executable `next_actions` carrying the same project and
corpus routing as the result. See [sessions](sessions.md).

## Result identity and evidence

Every result has a structured locator. Its identity is the tuple of corpus,
content ID, and resolution; linked-project and cross-corpus routing travel
with it. A bare content ID is meaningful only within one corpus.

Scores include a compact machine-readable explanation of the evidence that
contributed to the final rank: dense and sparse participation, reciprocal-rank
fusion, exact or identifier matches, intent boosts, optional reranking or
Matryoshka rescoring, graph expansion, and diversity selection. Results also
classify their provenance, such as production, test, generated, fixture,
vendor, documentation, migration, benchmark, or example. Production normally
ranks ahead of support material, while an explicit query for tests, fixtures,
or generated bindings reverses that preference.

Selection is novelty-aware across logical symbol, parent section, file,
package/module, resolution family, and provenance. The best exact result stays
first, but its claim, summary, stub, and full-body variants cannot consume the
whole response. Narrow queries may still return several genuinely distinct
members from one module.

## Bounds, pagination, and status

Survey enforces both per-result and total response budgets. Collection tools
use deterministic pagination and report `total`, `has_more`, `omitted_count`,
and the applied limit; excessive requested limits are clamped explicitly.

Every relevant query reports `status` (`ok`, `partial`, or `error`) separately
from index `completeness` (`complete`, `partial`, `stale`, or `unavailable`).
`absence_is_conclusive` is true only when the searched index can support a
negative conclusion. A partial fan-out keeps successful results and reports
the incomplete or failed corpus, a stable error code, retryability, and concise
retry guidance.

## Cross-corpus search

`corpus_ids` fans a query out across multiple corpora
([linked projects](../guides/configuration.md) or cloned repos), tags each
hit with `source_corpus`, and merges by score; `corpus_boost` applies
per-corpus multipliers. Completeness is reported per corpus, so one unavailable
member cannot turn a mixed-success search into a misleading empty success.
