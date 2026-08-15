# Sessions

Context is the scarce resource in agent work. ministr keeps a per-session
ledger of everything it has delivered, and uses it to never charge twice
for the same content.

## Corpus-aware delivery identity

A delivery is identified structurally by corpus ID, content ID, and resolution.
The corpus may be the primary project, a linked-project label, a daemon corpus,
or an Atlas source. The structured identity is serialized as data, never as an
ambiguous delimiter-joined string.

Different resolutions are independent deliveries because they carry different
information: an excerpt does not imply that the full section was delivered,
and a symbol outline does not imply that its body was delivered. Diversity
selection prevents those variants from monopolizing discovery results, while
the ledger still accounts for each representation honestly.

Sessions persisted before corpus-aware identities are migrated by scoping a
legacy bare content ID to the primary corpus and preserving the stored or
legacy resolution. This keeps old sessions usable without allowing a primary
delivery to suppress a colliding ID in another corpus.

## Dedup and delta delivery

- Search results already delivered at the same corpus and resolution are
  excluded before truncation. A repeated `ministr_survey` returns new material,
  with a `deduplicated_count` noting what was skipped and saved-token accounting
  reflecting the avoided delivery.
- Re-reading an unchanged section returns a short stub instead of the full
  text.
- Re-reading a changed section is re-delivered — the agent gets the new
  content precisely because it changed underneath.

The contract has an agent-side half: after dropping content from context,
call `ministr_dropped` with the delivered locator so that exact corpus and
resolution becomes eligible again. Other corpora and resolutions remain in the
ledger. To keep something referenceable in less space, `ministr_compress`
produces extractive summaries associated with the same locator.

## Coherence

When an indexed file changes after its content was delivered to a session,
subsequent tool responses carry `coherence_alerts` naming the affected
corpus-aware locators — the signal to re-read and pick up the delta. The
[freshness](freshness.md) sweep and this mechanism are two views of the
same change tracking.

## Honest accounting

`ministr_usage` reports what ministr has delivered this session. The
estimate is anchored to a configured window, not the model's real context
window, and the tool's own description says so: it is advisory, and agents
are told not to use it to conclude they are low on context.

The same report exposes deduplicated deliveries, saved tokens, and prefetch
outcomes. Prefetch keys include corpus identity. Hits, misses, issued and
unused prefetches, bytes/tokens and modeled latency saved, waste rate, and
per-strategy outcomes distinguish useful warming from speculative work. Saved
bytes/tokens and latency estimate backend work avoided by a cache hit; they are
not literal MCP response savings. The task-level payload gate measures bytes
and tokens crossing the MCP boundary separately.

## Session identity and recovery

Sessions are tracked per MCP connection and persisted, so delivery state
survives restarts. The MCP registry is the single deduplication owner for MCP
calls in both local and daemon-forwarded modes: it sends the same structured
exclusion identities through linked and fan-out routes and records each
delivery once. Direct daemon API clients continue to use daemon-owned session
ledgers. This separation avoids two partially synchronized shadows while
preserving the daemon session API for non-MCP consumers.
