# Prefetch Engine

The prefetch engine predicts what the agent will need next and pre-computes it, reducing response times from 50-200ms (cold retrieval) to <1ms (memory lookup).

## Why Prefetch Matters

Every tool call round-trip costs latency, tokens, and money. If iris can predict what the agent will ask for next and have it ready, the interaction feels instantaneous. The goal is a hit rate of >50% after the first 3 turns of a session.

## Prediction Strategies

The prefetch engine uses three deliberately simple heuristics. No LLM is in the loop — if prediction took 500ms of inference, it would defeat the purpose.

### Sequential Locality

When the agent reads section N of a document, pre-warm section N+1. Also pre-warm the parent document's summary (for navigation) and any sections that the current section cross-references.

This is the same principle as hardware prefetch streaming.

### Topical Locality

Maintain a running "topic vector" — a weighted average of the embedding vectors of the last K sections the agent accessed. Use this topic vector to find the nearest un-accessed sections and pre-warm them.

This is analogous to stride-based prefetching generalized to a high-dimensional embedding space.

### Structural Locality

If the agent accessed a claim within a section, pre-warm other claims in the same section. If the agent accessed a section within a document, pre-warm sibling sections. Walk up and sideways in the document tree.

## Cache

Pre-warmed results are stored in an in-memory LRU cache with a configurable size limit (default: 50 items). Each item includes the pre-computed text, token count, and relevance score.

## Configuration

```toml
[prefetch]
enabled = true      # Toggle prefetching on/off
cache_size = 50     # Max items in the prefetch cache
topic_window = 5    # Recent sections for topic vector
```

Prefetching can be disabled entirely with `enabled = false`. The cache size controls memory usage — each cached item holds the full text of a section or claim.

## Cross-Session Learning

Over time, iris accumulates data about access patterns:

- **Frequently accessed sections** get priority in prefetch caches
- **Sections consistently delivered together** are pre-bundled
- **Sections delivered but never referenced again** are deprioritized

This data is stored locally per corpus, fed by cross-session analytics.
