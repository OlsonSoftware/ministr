# iris

**Context cache controller for LLM agents.**

iris is an [MCP server](https://modelcontextprotocol.io) that manages your agent's context window the way a CPU cache controller manages L1 cache — tracking what the agent has seen, predicting what it needs next, and managing evictions when the budget runs low. It runs locally, embeds locally, and works with any MCP client.

## The Problem

Every major LLM advertises a context window measured in hundreds of thousands of tokens. But effective context — the portion where the model maintains reliable accuracy — is dramatically smaller. Research shows models degrade sharply once a threshold is crossed, and information placed in the middle of the context is systematically harder to retrieve (the "lost in the middle" effect).

**More tokens in the window does not mean better answers.** In many cases, it means worse ones. The bottleneck is not window size — it is what is *in* the window.

## The Insight

The context window is L1 cache, not memory. It shares every defining characteristic:

- **Small relative to total data** — an agent may have millions of documents but attend to ~100k tokens at a time
- **The only thing the processor uses** — out of window, out of mind
- **Expensive per unit** — every token costs compute, money, and latency
- **Quality depends on contents, not capacity**

Despite fifty years of cache management theory in computer architecture, there has been no equivalent of a cache controller for LLM context windows — until iris.

## How iris Helps

iris sits alongside any MCP-compatible agent as a sidecar process and provides:

| CPU Cache Concept | iris Equivalent | What It Does |
|---|---|---|
| Cache directory | Session shadow | Tracks what context the agent currently has |
| Cache line | Context unit | Atomic element: claim, section, or summary |
| Prefetcher | Speculative prefetch | Predicts what the agent needs next |
| Replacement policy | Relevance decay | Advises what to evict when the window is full |
| Cache coherence | Change detection | Notifies when underlying documents change |
| Multi-level cache | Multi-resolution index | Three levels: summaries, sections, claims |

## Key Differentiators

- **Not an agent runtime** — iris is a sidecar, not a framework. It works with any MCP client.
- **Not a vector database** — iris answers "what is the minimum additional context needed?" not just "what vectors are similar?"
- **Not classical RAG** — iris is stateful and multi-turn. It tracks what the agent already has, predicts next needs, and manages budget.
- **Fast** — warm responses in <1ms, cold retrieval in <50ms. Written in Rust with zero-copy where possible.
