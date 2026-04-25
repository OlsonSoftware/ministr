# ministr — Design Specification

**A context cache for LLM agents.**
*Don't waste your agent's scarcest resource.*

Version 0.1 | March 2026

> **This is a frozen design document, not a current reference.** It captures the original research and design rationale behind ministr. The implementation has evolved since v0.1: mechanism counts, prefetch strategy names, MCP tool surface, and resolution levels have all moved on. **For the current state of the system, read [the docs site](https://ministr.ai/docs)** — specifically [`/docs/architecture`](https://ministr.ai/docs/architecture), [`/docs/architecture-deep-dive`](https://ministr.ai/docs/architecture-deep-dive), and [`/docs/concepts`](https://ministr.ai/docs/concepts). This spec stays unchanged because it's the audit trail for *why* the system was built this way.

---

## How to read this document

This specification references current research, tools, and ecosystem developments that move fast. Throughout the document, you will find **[VALIDATE]** markers with suggested search queries. These are invitations to verify claims against current reality using web search before making implementation decisions. Technology in this space evolves on a monthly cadence; treat every factual assertion as something worth re-checking.

---

## 1. The problem

### 1.1 Context windows are a lie

Every major LLM advertises a context window measured in hundreds of thousands or millions of tokens. These numbers are architectural limits, not operational ones.

Research consistently shows that effective context — the portion of the window where the model maintains reliable accuracy — is dramatically smaller than advertised. Elvex's 2026 benchmarks found that a model claiming 200,000 tokens typically becomes unreliable around 130,000, with effective capacity at roughly 60–70% of the advertised maximum. The degradation is not gradual; models maintain good performance until a threshold, then quality drops sharply.

**[VALIDATE]** Search: `context window effective capacity 60-70% advertised 2026 benchmark`

More concerning, a peer-reviewed study by Paulsen (2025) tested real-world tasks (not just needle-in-a-haystack retrieval) and found that most models suffered severe accuracy degradation by just 1,000 tokens of context for complex reasoning tasks like sorting and multi-step aggregation. The maximum effective context window (MECW) fell short of the maximum context window (MCW) by as much as 99%.

**[VALIDATE]** Search: `arxiv "Context Is What You Need" Paulsen maximum effective context window`

The "lost in the middle" effect, first documented by Liu et al. (2023), persists in 2026 architectures. Information placed at positions 10–50% into the context is systematically harder for models to retrieve than information at the beginning or end. This creates a U-shaped recall curve that no amount of context window expansion has eliminated.

**[VALIDATE]** Search: `"lost in the middle" Liu 2023 U-shaped recall 2025 2026 still present`

A December 2025 paper by Hossain et al. further demonstrated that performance degradation under large context is non-linear and tied to KV cache growth, with Mixture-of-Experts architectures exhibiting unique failure modes at scale.

**[VALIDATE]** Search: `arxiv "Context Discipline and Performance Correlation" Hossain KV cache degradation`

The implication: **more tokens in the window does not mean better answers.** In many cases, it means worse ones. The bottleneck is not window size. It is what is *in* the window.

### 1.2 The context window is L1 cache, not memory

A March 2026 arxiv paper titled "The Missing Memory Hierarchy: Demand Paging for LLM Context Windows" (Mason, 2026) articulates a framing that is central to ministr's design:

> "The context window of a large language model is not memory. It is L1 cache: a small, fast, expensive resource that the field treats as the entire memory system. There is no L2, no virtual memory, no paging."

**[VALIDATE]** Search: `arxiv "The Missing Memory Hierarchy" "demand paging" LLM context Pichay 2026`

This is not a metaphor. It is a structural observation. The context window shares every defining characteristic of an L1 cache:

- **Small relative to total data.** An agent may have access to millions of documents, but can attend to at most ~100k tokens at a time.
- **The only thing the processor uses.** The LLM cannot reason about information that is not in its context. Out of window, out of mind.
- **Expensive per unit.** Every token in the window costs compute (attention is O(n²) or O(n·d) depending on architecture), money (per-token API pricing), and latency.
- **Quality depends on contents, not capacity.** A cache full of irrelevant data is worse than a smaller cache with the right data.

Despite fifty years of cache management theory in computer architecture, the AI industry has no equivalent of a cache controller for LLM context windows. Agents either stuff everything they can find into the window (the equivalent of thrashing), or make a single vector-search query and hope the results are relevant (the equivalent of a random cache replacement policy).

### 1.3 RAG is one-shot cache loading

Classical Retrieval-Augmented Generation (RAG) — embed query, search vector index, stuff top-k chunks into the prompt — is essentially a single cache fill operation. It has no concept of:

- **What's already loaded.** Every query starts from scratch, frequently returning context the agent already has.
- **What will be needed next.** There is no prediction, no prefetching. Every cache miss (missing context) requires a full round-trip.
- **Budget constraints.** RAG returns a fixed number of chunks regardless of how much window space is actually available.
- **Coherence.** When the underlying documents change, previously retrieved context becomes stale with no notification.

The industry is recognizing this limitation. RAGFlow's 2025 year-end review describes the evolution from RAG as a pattern toward RAG as a "context engine." Gartner has called 2026 "the year of context." The term "context engineering" has emerged to describe the broader discipline of managing what information reaches the model.

**[VALIDATE]** Search: `"context engineering" 2026 Gartner "year of context" RAG evolving`

But the tooling has not caught up. Frameworks like Letta (MemGPT), Google ADK, and AWS AgentCore Memory each address parts of this problem, but they are locked into specific runtimes or cloud platforms. There is no standalone, agent-agnostic, fast, lightweight tool that manages context as a scarce resource.

**[VALIDATE]** Search: `Letta MemGPT vs Mem0 vs Google ADK agent memory 2026 comparison`

### 1.4 What agents actually need

In an agentic workflow (which is the dominant deployment pattern for LLMs in 2026), the interaction between an agent and its knowledge source is not a single query. It is a multi-turn conversation:

1. Agent receives a task.
2. Agent realizes it needs information.
3. Agent calls a tool to retrieve context.
4. Agent reads the context, begins working.
5. Agent realizes it needs *more* or *different* information.
6. Agent calls the tool again.
7. Steps 4–6 repeat multiple times.
8. Agent produces a final output.

Every round-trip in this loop costs latency, tokens, and money. The quality of the final output depends not on any single retrieval, but on the *cumulative* quality of context across the entire session. This is a fundamentally different problem from single-query retrieval, and it demands a fundamentally different system.

---

## 2. The idea

### 2.1 ministr in one sentence

ministr is a Rust-native MCP server that serves context to an LLM agent the way an L1 cache serves the CPU — with state tracking, predictive prefetching, budget awareness, and coherence. ministr doesn't (and can't) manipulate the agent's context window directly; it manages its own output — what it sends, when, and at what resolution — so the agent's window stays clear of redundant reads.

### 2.2 The cache analogy

Every design decision in ministr derives from a single structural analogy:

| CPU cache concept | ministr equivalent | What it does |
|---|---|---|
| Cache directory | Session shadow | Tracks what context the agent currently has in its window |
| Cache line | Context unit | The atomic element of context: a claim, a section, or a summary |
| Prefetcher | Speculative prefetch | Predicts what the agent will need next based on trajectory |
| Replacement policy | Relevance decay model | Advises the agent on what to evict when the window is full |
| Cache coherence protocol | Change detection | Notifies the agent when underlying documents change, invalidating stale context |
| Multi-level cache (L1/L2/L3) | Multi-resolution index | Three levels of detail: summaries, sections, claims |
| Cache hit | Warm response | <1ms because the answer was already pre-computed |
| Cache miss | Cold retrieval | Full embed → search → rank pipeline, typically 50–200ms |

This analogy is not decorative. It constrains every design choice. When evaluating a feature request, the question is: "Does a cache controller do this?" If not, it is out of scope.

### 2.3 How ministr differs from everything else

**ministr is NOT Letta/MemGPT.** Letta is an agent runtime — you build your agent inside it, and it manages context as part of its execution loop. ministr is a sidecar — it sits alongside any agent that speaks MCP and serves context on demand. Letta is the operating system; ministr is the cache controller that the operating system calls.

**[VALIDATE]** Search: `Letta MemGPT agent runtime Python "operating system" architecture 2026`

**ministr is NOT a vector database.** Qdrant, Pinecone, and Weaviate are storage engines. They answer "what vectors are similar to this one?" ministr answers "given what this agent already knows and what it is trying to do, what is the minimum additional context needed to make progress?"

**ministr is NOT classical RAG.** RAG is a single-shot query-response pattern. ministr is a stateful, multi-turn context management service. RAG doesn't know what the agent already has; ministr tracks it. RAG doesn't predict next needs; ministr prefetches. RAG ignores budget; ministr manages it.

**ministr is NOT the Pichay system** described in Mason (2026). Pichay operates at the *token level* as a transparent proxy between client and API, evicting stale tool outputs from the raw message stream. ministr operates at the *knowledge level* as an MCP tool server, deciding which information from a document corpus should become tokens in the first place. They are complementary: Pichay manages the plumbing; ministr manages the water supply.

---

## 3. Architecture

### 3.1 Deployment model

ministr runs as a standalone process that any MCP-compatible agent can connect to. It is not embedded inside the agent, not a library linked into the agent's binary, and not a cloud service. It is a local sidecar process, the same way a language server (LSP) sits alongside an editor.

<p align="center">
  <img src="docs/src/assets/deployment-model.svg" alt="Deployment model: MCP client (agent) connects to ministr, which reads from a local document corpus" width="780">
</p>

The MCP protocol (donated to the Linux Foundation's Agentic AI Foundation in December 2025, co-founded by Anthropic, Block, and OpenAI) is the universal interface. ministr does not need to know anything about the agent's internals. It only needs to receive tool calls and return context.

**[VALIDATE]** Search: `MCP Model Context Protocol Linux Foundation Agentic AI Foundation December 2025`

### 3.2 The five mechanisms

ministr combines five mechanisms that, individually, exist in fragments across the research landscape but have never been unified into a single system:

#### Mechanism 1: Session shadow

When ministr delivers context to an agent, it records exactly what was delivered — the specific sections, claims, and summaries, along with the turn number. This creates a "shadow" of what is currently in the agent's context window.

The shadow enables three capabilities that no existing retrieval system provides:

- **Deduplication.** If the agent asks about "authentication" and ministr already provided the auth docs three turns ago, ministr does not return the same text. It recognizes the overlap and either says "you already have this" or provides only what has changed.
- **Delta updates.** "Section 3.2 was updated since you last read it. Here is what changed." This is impossible without knowing what was previously delivered.
- **Eviction estimation.** Based on how many turns have passed and how much new context has been added, ministr estimates what the agent has likely dropped from its window (due to context window limits or summarization). This allows ministr to re-deliver critical context that may have been evicted.

The shadow is a lightweight data structure (a set of content hashes with turn numbers and token counts) stored in memory for the active session and persisted to SQLite for session recovery.

#### Mechanism 2: Multi-resolution document index

Classical RAG destroys document structure by splitting text into fixed-size chunks. ministr preserves it by indexing at three simultaneous resolutions:

**Level 1: Summaries.** For every document and every major section within a document, ministr pre-generates a compressed summary (typically 50–100 tokens for a section, 200–400 for a document). These summaries are generated at ingestion time using a small local model or heuristic extraction. They give the agent a "table of contents" view of the knowledge base.

**Level 2: Sections.** The document's natural structure — headings, paragraphs, code blocks, tables — is preserved as discrete, addressable units. Each section retains its heading hierarchy (e.g., "Chapter 3 > Section 3.2 > Subsection: Error Handling"). Sections are the primary unit of retrieval.

**Level 3: Claims.** Within each section, ministr extracts atomic factual statements — individual claims that can stand alone. "The auth service uses JWT tokens with RS256 signing." "Rate limits are set to 100 requests per minute per API key." Claims are the highest-resolution unit, used when the agent needs a specific fact rather than surrounding context.

These three levels are not separate indexes. They form a tree: documents contain sections contain claims. Each level has its own embedding vector, enabling search at any granularity. The tree structure enables navigation: an agent can survey at the summary level, read at the section level, and extract at the claim level.

This hierarchical approach aligns with the "cross-granularity retrieval" pattern identified in recent chunking strategy research, where indexing at atomic (sentence-level) units and assembling context at query time outperforms fixed-size chunking.

**[VALIDATE]** Search: `cross-granularity retrieval sentence-level atomic chunking query-time assembly 2025 2026`

#### Mechanism 3: Progressive disclosure via MCP tools

Instead of a single `search(query) → chunks` interface, ministr exposes four MCP tools that give the agent explicit control over retrieval depth:

**`ministr_survey(query)`** — Returns high-level summaries of relevant document areas. Costs ~200 tokens. Designed to orient the agent before it commits to reading anything in detail. Analogous to scanning chapter titles in a book.

**`ministr_read(section_id)`** — Returns the full text of a specific section identified by its hierarchical ID (e.g., `docs/auth.md#error-handling`). The agent chooses what to read based on what the survey revealed.

**`ministr_extract(query, section_id)`** — Returns only the specific claims within a section that are relevant to the query. This is for surgical precision: "I know the answer is somewhere in this section; give me just the facts."

**`ministr_related(claim_id)`** — Given a specific claim, returns other claims that reference, depend on, or contradict it. This enables the agent to follow chains of reasoning across documents: "The rate limit is 100/min" → "Rate limit exceptions require an API key with the 'elevated' tier" → "Elevated tier keys are provisioned by the platform team."

This four-tool interface mirrors how a human researcher navigates a knowledge base: survey, read, extract, connect. It gives the agent agency over its own context loading, rather than forcing it to accept whatever a search algorithm returns.

#### Mechanism 4: Speculative prefetch

Based on the sequence of tool calls the agent has made so far, ministr predicts what the agent will likely need next and pre-computes it.

The prediction model is deliberately simple and heuristic-based (no LLM in the loop for prefetching — that would defeat the latency purpose):

- **Sequential locality.** If the agent read sections 1, 2, 3 of a document, pre-warm section 4. (Same principle as hardware prefetch streaming.)
- **Topical locality.** If the agent is reading about "authentication," pre-warm other security-related sections from the index. (Based on embedding similarity of section summaries.)
- **Task pattern matching.** If the agent's message history matches a known task pattern (e.g., "debugging an API error"), pre-warm sections that were useful in previous similar sessions. (Requires cross-session analytics, a later-phase feature.)

When a prefetched result is served, the response time drops from 50–200ms (cold retrieval with embedding + search) to <1ms (memory lookup). This is the difference between the agent needing one tool call per turn versus three, and it compounds across an entire session.

#### Mechanism 5: Context budget management

ministr accepts a `max_context_tokens` configuration parameter that represents the agent's total context window budget. As the session shadow grows, ministr tracks cumulative token usage and provides active guidance:

- **Budget-aware responses.** When ministr returns context, it includes a `tokens_used` count and a `budget_remaining` estimate. The agent (or its orchestration framework) can use this to make informed decisions.
- **Eviction recommendations.** When the budget is approaching capacity, ministr can be asked: "What should I drop?" It ranks currently-shadowed content by recency, relevance to the current task, and dependency (content that other content depends on is retained longer). It returns a list of section IDs to evict, along with compressed summaries that can replace them.
- **Compression on demand.** An agent can call `ministr_compress(section_id)` to get a summary of a section it wants to evict. This preserves the gist of the information while freeing window space.

This mechanism has no analogue in any existing retrieval system. RAG does not know the agent's budget. Vector databases do not track cumulative token usage. Letta manages budget internally but does not expose it as a service.

### 3.3 What ministr does NOT do

Following the cache controller analogy strictly, ministr excludes:

- **LLM inference.** ministr does not generate answers. It provides context to agents that generate answers. (A cache controller does not execute instructions; it feeds data to the processor.)
- **Agent orchestration.** ministr does not decide when to retrieve or what task the agent is working on. The agent calls ministr when it needs context. (A cache controller responds to memory requests; it does not initiate them.)
- **Multi-tenancy or authentication.** ministr serves one agent (or one user's agents) at a time. Enterprise multi-tenancy belongs in a layer above.
- **Document creation or editing.** ministr is read-only over the document corpus. It indexes and serves; it does not modify source documents.

---

## 4. The multi-resolution index in detail

### 4.1 Ingestion pipeline

When ministr ingests a document, it performs the following steps:

**Step 1: Parse and structure.** The document is parsed into a structural tree that preserves the author's original organization: headings, paragraphs, code blocks, tables, lists. Each structural element becomes a node in the tree, tagged with its position, depth, and type. The parser is format-aware — Markdown headings, HTML sections, PDF page boundaries, and code function definitions each produce appropriate structural nodes.

**Step 2: Section identification.** The structural tree is segmented into sections — coherent, author-defined units of text. A section is typically the content under a heading, but heuristics handle documents without clear headings (e.g., plain text split at paragraph boundaries, code files split at function/class boundaries). Each section receives a stable, human-readable ID based on its heading hierarchy.

**Step 3: Claim extraction.** Within each section, ministr extracts atomic claims — single factual statements that can stand alone. This can be done via:
- **Heuristic extraction:** Split on sentence boundaries, filter for statements containing named entities, numbers, or specific assertions. Fast, no model required.
- **Model-assisted extraction:** Use a small local model (via `fastembed` or a small GGUF model) to identify and normalize claims. Higher quality, higher ingestion cost.

**[VALIDATE]** Search: `fastembed rust crate version 2026 embedding models reranking`

The extraction mode is configurable per corpus. For a fast first pass, heuristic extraction is sufficient. For high-value corpora (legal documents, API specifications, compliance docs), model-assisted extraction is worth the cost.

**Step 4: Embedding.** Each node in the tree — summary, section, and claim — receives its own embedding vector. ministr uses `fastembed` (a Rust crate wrapping ONNX Runtime with built-in support for models like all-MiniLM-L6-v2, BGE, and others) for embedding. The three-level embedding strategy means the same query can match at any resolution: a broad query matches summaries, a specific query matches claims.

**Step 5: Summary generation.** For each section and each document, ministr generates a compressed summary. The default strategy is extractive: select the top-k most information-dense sentences (measured by TF-IDF score relative to the section) and concatenate them. An optional mode uses a small local model for abstractive summarization.

**Step 6: Index and store.** All embeddings are inserted into an HNSW vector index. All text, metadata, and structural relationships are stored in SQLite. The vector index is memory-mapped for instant loading on startup.

**[VALIDATE]** Search: `hnswlib-rs crate Rust HNSW pure-Rust concurrent deletion 2025 2026`

### 4.2 Incremental updates

ministr tracks document file hashes. When a watched directory changes:

1. Modified files are re-parsed and re-indexed. Only changed sections are re-embedded and re-inserted.
2. The session shadow is consulted: if any active session references a section that has changed, ministr generates a "coherence notification" — a delta describing what changed. This notification is available to the agent on its next tool call.
3. Deleted files have their sections and claims removed from the index and marked as invalidated in any active session shadows.

This is directly analogous to a cache coherence protocol: when the backing store changes, cached copies are invalidated and updated.

---

## 5. The session shadow in detail

### 5.1 Data structure

The session shadow is a lightweight, in-memory structure that tracks the state of a single agent session:

```
Session {
    id: SessionId,
    created_at: Timestamp,
    agent_context_budget: usize,          // max tokens the agent can hold

    delivered: BTreeMap<TurnNumber, Vec<DeliveredItem>>,  // what was sent, when

    estimated_window: Vec<WindowSlot>,    // what we think is still in the agent's window
    total_tokens_delivered: usize,
    estimated_tokens_evicted: usize,

    prefetch_cache: HashMap<ContentId, PrefetchedResult>,  // pre-warmed results
    trajectory: Vec<TopicSignal>,         // sequence of topics the agent has explored
}

DeliveredItem {
    content_id: ContentId,     // links to a summary, section, or claim
    resolution: Resolution,    // Summary | Section | Claim
    token_count: usize,
    turn_delivered: usize,
    content_hash: u64,         // for change detection
}
```

### 5.2 Window estimation

ministr does not have direct access to the agent's actual context window (MCP does not expose this). Instead, it maintains an *estimate* based on:

- **Cumulative token count.** Every item delivered is tracked by size. When cumulative delivery exceeds the agent's declared budget, ministr assumes older items have been evicted (FIFO assumption, configurable to LRU).
- **Agent behavior signals.** If the agent re-asks for something ministr already delivered (a "fault"), ministr infers that the item was evicted and updates the shadow accordingly. This is directly analogous to a page fault in virtual memory.
- **Explicit signals.** An agent can optionally call `ministr_evicted(content_ids)` to explicitly tell ministr what it dropped, improving the shadow's accuracy.

The window estimate does not need to be perfect. Even a rough approximation prevents the most wasteful failure mode (re-delivering identical context every turn) while the fault-based correction mechanism converges on accuracy over time.

### 5.3 Cross-session learning

Over time, ministr accumulates data about which context was useful across sessions:

- **Frequently accessed sections** get priority in prefetch caches.
- **Sections that are consistently delivered together** are pre-bundled.
- **Sections that are delivered but never referenced again** are deprioritized in future rankings.

This is analogous to hardware cache profiling: observing access patterns to tune prefetch and replacement policies. The data is stored locally (no cloud dependency) and is per-corpus.

---

## 6. The prefetch engine in detail

### 6.1 Prediction heuristics

Prefetch in ministr is deliberately based on simple, fast heuristics rather than LLM-powered prediction. The reason is latency: if predicting the next need takes 500ms of LLM inference, you have not saved any time over just retrieving on demand. The prefetcher must operate in <5ms to be useful.

**Sequential prefetch.** When the agent reads section N of a document, pre-embed and pre-rank section N+1. Also pre-warm the parent section's summary (for navigation) and any sections that the current section cross-references.

**Topical prefetch.** Maintain a running "topic vector" — a weighted average of the embedding vectors of the last K sections the agent accessed. Use this topic vector to find the nearest un-accessed sections in the index and pre-warm them. This is analogous to stride-based prefetching in hardware, generalized to a high-dimensional embedding space.

**Structural prefetch.** If the agent accessed a claim within a section, pre-warm other claims in the same section. If the agent accessed a section within a document, pre-warm sibling sections. Walk up and sideways in the document tree.

### 6.2 Cache warming

Pre-warmed results are stored in an in-memory LRU cache with a configurable size limit (default: 50 items). Each item includes the pre-computed text, token count, and relevance score so that when the agent makes the request, the response can be assembled without touching the vector index or SQLite.

A cache hit (agent asks for something that was prefetched) is served in <1ms. A cache miss (agent asks for something unexpected) falls through to the full retrieval pipeline (typically 50–200ms depending on corpus size and embedding model speed).

The goal is a hit rate of >50% after the first 3 turns of a session, based on the observation that agent behavior within a session is topically concentrated.

---

## 7. Context budget management in detail

### 7.1 The problem of invisible eviction

The most pernicious issue in multi-turn agent workflows is invisible context eviction. As the agent accumulates context over many turns, older content silently falls out of the window. The agent does not know what it has lost. It may hallucinate facts it "remembers" from earlier turns that are no longer in context. It may ask the user to repeat information. It may produce contradictory outputs because it no longer has access to the constraints established earlier.

This is the AI equivalent of cache thrashing — the working set exceeds the cache size, and performance collapses.

### 7.2 ministr's approach

When an agent's estimated window usage exceeds a configurable threshold (default: 80% of `max_context_tokens`), ministr enters "pressure mode." In pressure mode:

- **Responses are automatically compressed.** Instead of returning full sections, ministr returns claim-level extracts by default, reducing token count by 60–80%.
- **Eviction recommendations are attached to every response.** ministr identifies the delivered content most likely to be safe to drop (based on recency, relevance decay, and dependency analysis) and includes these recommendations in tool call responses.
- **Summaries replace evicted content.** When recommending eviction, ministr simultaneously provides a compressed summary of each evicted item (typically 10–20% of the original token count) that the agent can retain as a placeholder.

This is analogous to how an OS under memory pressure swaps pages to disk and replaces them with compact metadata — the information is not lost, but it occupies less of the scarce resource, and can be faulted back in on demand.

---

## 8. MCP interface specification

### 8.1 Tools

ministr exposes the following MCP tools. Tool definitions follow the MCP 2025-11-25 specification.

**[VALIDATE]** Search: `MCP specification 2025-11-25 tools resources JSON-RPC`

**`ministr_survey`** — Orient the agent within the knowledge base.
- Input: `{ query: string, max_results?: number }`
- Output: `{ areas: [{ id, title, summary, relevance_score, token_count }], budget_status: { used, remaining, pressure } }`
- Typical token cost: 100–300 tokens
- Expected latency: <50ms cold, <5ms warm

**`ministr_read`** — Read a specific section in full.
- Input: `{ section_id: string }`
- Output: `{ text: string, heading_path: string[], token_count: number, claims_available: number, budget_status }`
- Typical token cost: 200–2000 tokens (varies by section length)
- Expected latency: <10ms (sections are pre-loaded in memory)

**`ministr_extract`** — Get only relevant claims from a section.
- Input: `{ query: string, section_id: string, max_claims?: number }`
- Output: `{ claims: [{ id, text, relevance_score }], budget_status }`
- Typical token cost: 50–500 tokens
- Expected latency: <50ms cold, <5ms warm

**`ministr_related`** — Follow dependency chains between claims.
- Input: `{ claim_id: string, relation_types?: ["references", "contradicts", "depends_on", "updates"] }`
- Output: `{ related: [{ claim_id, text, relation_type, source_section }], budget_status }`
- Typical token cost: 50–300 tokens
- Expected latency: <20ms

**`ministr_compress`** — Get a compressed summary of content the agent wants to evict.
- Input: `{ content_ids: string[] }`
- Output: `{ summaries: [{ original_id, summary, original_tokens, compressed_tokens }] }`

**`ministr_budget`** — Get the current budget status and eviction recommendations.
- Input: `{}`
- Output: `{ total_budget, estimated_used, estimated_remaining, pressure_level, eviction_candidates: [{ id, reason, tokens_recoverable, replacement_summary }] }`

**`ministr_evicted`** — Tell ministr what the agent explicitly dropped (improves shadow accuracy).
- Input: `{ content_ids: string[] }`
- Output: `{ acknowledged: true }`

### 8.2 Resources

ministr also exposes MCP resources for metadata access:

- `ministr://status` — Index statistics: document count, section count, claim count, index size.
- `ministr://corpus/{path}` — Metadata about a specific source document (title, sections, last modified).

### 8.3 Notifications

ministr uses MCP notifications (server-initiated messages) for:

- **Coherence alerts.** When a watched file changes and the change affects content in the session shadow, ministr pushes a notification: `{ type: "coherence_alert", changed_sections: [...], stale_content_ids: [...] }`.

---

## 9. Storage and persistence

### 9.1 On-disk layout

```
~/.ministr/
├── config.toml                     # Global configuration
└── corpora/
    └── <corpus-name>/
        ├── meta.toml               # Corpus config (source dirs, embedding model, etc.)
        ├── content.db              # SQLite: sections, claims, summaries, metadata
        ├── vectors.hnsw            # Memory-mapped HNSW index
        ├── vectors.meta            # Index metadata (dimensions, count, params)
        ├── file_hashes.json        # For incremental re-indexing
        └── sessions/
            └── <session-id>.json   # Persisted session shadows (for recovery)
```

### 9.2 SQLite schema (conceptual)

The content database stores the document tree and its three resolution levels. The schema captures parent-child relationships between documents, sections, and claims, along with source file provenance, heading hierarchy, and pre-generated summaries. Each node in the tree has an associated embedding stored in the HNSW index (referenced by the same ID).

Session data is stored separately from corpus data so that a corpus can be shared across multiple agents or sessions without interference.

---

## 10. Dependency strategy

ministr's dependency philosophy: use the best existing Rust crates for solved problems, build custom only for the novel mechanisms (session shadow, prefetch engine, budget manager).

**For embeddings:** `fastembed` (v5+). This crate provides local ONNX-based embedding with support for dozens of models, reranking, and sparse embeddings. It is maintained by the Qdrant team and has 23k+ monthly downloads. No reason to build custom.

**[VALIDATE]** Search: `fastembed crate Rust version 5 features supported models 2026`

**For vector search:** `hnswlib-rs` (pure Rust, concurrent reads/writes, supports deletion, decouples graph from storage) or `hnsw_rs` (older but proven). The choice should be validated against current benchmarks.

**[VALIDATE]** Search: `Rust HNSW crate comparison hnswlib-rs hnsw_rs benchmark 2026`

**For MCP protocol:** `rmcp` (the official Rust MCP SDK) or `mcpr`. The Rust MCP ecosystem is young; check which crate has best spec compliance.

**[VALIDATE]** Search: `Rust MCP SDK rmcp mcpr prism-mcp-rs crate comparison 2026`

**For document parsing:** `comrak` (Markdown), `scraper` + `html2text` (HTML), `pdf-extract` or `lopdf` (PDF), `tree-sitter` (source code). Each format gets a pluggable parser.

**For storage:** `rusqlite` for structured data, `memmap2` for memory-mapped vector indexes, `notify` for file watching.

**For HTTP/MCP transport:** `axum` for an optional HTTP transport alongside the standard stdio MCP transport.

---

## 11. Performance targets

### 11.1 Latency

| Operation | Target (p50) | Target (p99) |
|---|---|---|
| Warm survey (prefetched) | <1ms | <5ms |
| Cold survey (full pipeline) | <50ms | <200ms |
| Section read (from memory) | <1ms | <5ms |
| Claim extraction (cold) | <50ms | <150ms |
| Related claims traversal | <10ms | <50ms |
| Budget status | <1ms | <1ms |

### 11.2 Throughput

| Corpus size | Ingestion time | Index memory | Prefetch hit rate (after turn 3) |
|---|---|---|---|
| 1,000 sections | <30s | ~50MB | >40% |
| 10,000 sections | <5 min | ~200MB | >50% |
| 100,000 sections | <30 min | ~1GB | >60% |

### 11.3 Binary size

| Configuration | Target |
|---|---|
| Minimal (fastembed + HNSW + SQLite) | ~25MB |
| Full (all parsers + all features) | ~60MB |

---

## 12. Phased roadmap

### Phase 0: Foundation (weeks 1–3)

- Cargo workspace: `ministr-core`, `ministr-mcp`, `ministr-cli`
- MCP server scaffolding using `rmcp` (stdio transport)
- SQLite schema and storage layer
- Document parser trait + Markdown parser
- Basic section-level indexing (no claims, no summaries yet)
- `ministr_read` tool working end-to-end

**Milestone:** An agent can point ministr at a folder of Markdown files and read individual sections via MCP.

### Phase 1: Multi-resolution index (weeks 4–7)

- Embedding pipeline using `fastembed`
- HNSW vector index with persistence
- `ministr_survey` tool (vector search over section embeddings)
- `ministr_extract` tool (claim extraction — heuristic mode)
- Summary generation (extractive)
- HTML and PDF parsers
- Incremental re-indexing

**Milestone:** An agent can survey, read, and extract from a mixed-format document corpus.

### Phase 2: Session intelligence (weeks 8–12)

- Session shadow implementation
- Deduplication (don't return what agent already has)
- Budget tracking and pressure mode
- `ministr_budget` and `ministr_compress` tools
- `ministr_evicted` tool (agent feedback)
- Basic prefetch engine (sequential + topical locality)
- Warm/cold response metrics

**Milestone:** ministr tracks session state, deduplicates, manages budget, and achieves >30% prefetch hit rate.

### Phase 3: Polish and release (weeks 13–16)

- `ministr_related` tool (claim dependency traversal)
- File watching with coherence notifications
- Cross-session analytics (frequently-accessed sections, co-access patterns)
- Model-assisted claim extraction (optional feature flag)
- Comprehensive documentation (mdBook)
- Pre-built binaries for Linux (x86_64, aarch64), macOS (Apple Silicon), Windows
- Benchmark suite with reproducible evaluation

**Milestone:** v0.1.0 public release.

### Post-v1 directions

- **Source code awareness** via tree-sitter: function-level indexing for codebases.
- **Multi-corpus support:** search across multiple knowledge bases with per-corpus ranking.
- **Reranking:** integrate `fastembed`'s built-in cross-encoder reranking for improved precision.
- **HTTP transport:** optional HTTP/SSE transport in addition to stdio, for remote deployment.
- **Agent feedback loop:** agents explicitly mark which context was useful; ministr uses this to improve future retrieval and prefetch.
- **Conversation-aware retrieval:** use the full agent conversation (not just the latest tool call) to improve survey relevance.
- **WebAssembly target:** compile ministr-core to WASM for browser-based deployments.

---

## 13. Risks and mitigations

| Risk | Severity | Mitigation |
|---|---|---|
| **Session shadow accuracy degrades over long sessions.** Without direct access to the agent's actual window, the shadow estimate drifts. | High | Fault-based correction (re-requests indicate eviction). Explicit `ministr_evicted` signal. Conservative eviction estimates. Regular accuracy audits in benchmarks. |
| **Prefetch hit rate too low to justify memory cost.** If the agent's behavior is unpredictable, pre-warming is wasted work. | Medium | Default prefetch cache is small (50 items). Hit rate is measured per session. Prefetch can be disabled entirely via config. Simple heuristics fail gracefully (wasted memory, not wrong answers). |
| **Claim extraction quality is unreliable.** Heuristic extraction may produce incomplete or noisy claims. | Medium | Claim extraction is a progressive enhancement, not a requirement. Agents can always fall back to section-level retrieval. Model-assisted extraction is available for high-value corpora. Claims are always linked to their source section for verification. |
| **MCP ecosystem instability.** The MCP spec and Rust SDKs are evolving rapidly. | Medium | Abstract the MCP transport behind a trait. Support both `rmcp` and raw JSON-RPC. Pin to a specific spec version (2025-11-25) and track the 2026 roadmap. **[VALIDATE]** Search: `MCP 2026 roadmap specification release June` |
| **Embedding model quality limits retrieval.** Small local models may produce embeddings that miss semantic nuances. | Medium | Default to `all-MiniLM-L6-v2` (proven, fast, 384d). Support model swapping via config. Multi-resolution indexing partially compensates — if embeddings miss at one level, they may hit at another. |
| **Scope creep toward becoming a full agent framework.** | High | The cache controller analogy is the scope boundary. Every feature request is evaluated: "Does a cache controller do this?" If not, it is out of scope. ministr provides data; the agent provides intelligence. |

---

## 14. Open questions

1. **How should ministr handle multi-modal documents?** PDFs with images, diagrams, tables. For v0.1, ministr extracts text only. Should future versions integrate vision models for image understanding, or is that a different system's job? **[VALIDATE]** Search: `multi-modal RAG document image table extraction 2026 approaches`

2. **Should ministr manage conversation history?** Currently, ministr only manages knowledge retrieval from a document corpus. Some agent frameworks also struggle with conversation history management (summarizing old turns, retaining key decisions). Should ministr's budget management extend to non-document context? The cache controller analogy says no (that's a different cache), but the user's pain says maybe.

3. **How to handle claims that contradict each other across documents?** If two documents make conflicting claims (e.g., different version numbers), should ministr detect and surface the contradiction? This adds complexity but could be high-value for compliance and audit use cases.

4. **What is the right default embedding model in March 2026?** The landscape shifts constantly. `all-MiniLM-L6-v2` is proven but aging. Newer models like `nomic-embed-text-v1.5` and `bge-small-en-v1.5` may offer better quality. The `fastembed` crate now supports Qwen3 embeddings via candle. **[VALIDATE]** Search: `best small embedding model 2026 comparison MiniLM BGE nomic`

5. **Should ministr support remote corpus sources?** Currently, ministr indexes local files. But agents increasingly work with cloud data (Google Drive, Notion, GitHub). Should ministr support remote ingestion, or should that be handled by separate tools that materialize documents locally? The cache controller analogy suggests the latter (a cache controller doesn't fetch from the network; the memory bus does).

6. **How does ministr interact with the Pichay-style systems?** If an agent uses both ministr (for knowledge retrieval) and a Pichay-like proxy (for token-level window management), how should they coordinate? Is there a shared budget model? This is unexplored territory. **[VALIDATE]** Search: `arxiv Pichay demand paging LLM context proxy 2026`

---

## 15. Why now, why Rust, why this

### Why now

The convergence of three developments makes this the right moment:

1. **MCP standardization.** The protocol is now governed by the Linux Foundation with backing from every major AI company. Building on MCP means ministr is immediately compatible with every major agent platform. This was not possible a year ago.

2. **"Context engineering" as a recognized discipline.** The industry has moved past "just use RAG" and acknowledged that context management is a distinct engineering problem requiring dedicated tooling. The demand exists; the tooling does not.

3. **The research is mature enough to implement.** The cache hierarchy analogy (Mason 2026), session-aware retrieval patterns (Google ADK), multi-resolution indexing (KohakuRAG, DeepRead), and context budget management concepts have all been published and validated in research. What is missing is a unified, production-quality implementation.

### Why Rust

The hot path in ministr — session shadow lookup, prefetch cache check, vector search, budget calculation — must be invisible to the agent. If a tool call adds 200ms of latency, agents will make fewer calls and get worse context. The target is <5ms for warm responses, which requires:

- In-memory data structures with no GC pauses.
- Memory-mapped file I/O for instant index loading.
- Zero-copy where possible.
- Predictable latency under load.

Rust provides all of these. Python cannot. Go comes close on latency but lacks Rust's memory-mapping ergonomics and the `fastembed` / HNSW ecosystem. The Rust MCP SDK ecosystem is already viable with multiple options.

### Why this specific design

Every design choice can be traced to the cache controller analogy:

- **Session shadow** = cache directory.
- **Multi-resolution index** = cache line hierarchy (word, block, page).
- **Progressive disclosure tools** = load/store instructions at different granularities.
- **Speculative prefetch** = hardware prefetcher.
- **Budget management** = replacement policy + pressure handling.
- **Coherence notifications** = cache coherence protocol.

The analogy is not decoration. It is a constraint that prevents scope creep, guides prioritization, and provides a proven theoretical framework for every mechanism in the system. Cache controllers are among the most well-studied subsystems in computer architecture. ministr applies fifty years of that theory to a new domain.

---

*This is a living document. Last updated: March 21, 2026.*
*All [VALIDATE] markers indicate claims that should be checked against current information before implementation decisions are made.*