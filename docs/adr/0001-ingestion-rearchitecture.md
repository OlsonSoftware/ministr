# ADR 0001 — Local ingestion rearchitecture (SOLID, bounded, backpressured)

- **Status:** Accepted (design gate) — 2026-06-01
- **Scope:** `ministr-core` ingestion + embedding + storage (the local, MIT stack)
- **Supersedes:** the `INDEXING_SEMAPHORE(1)` band-aid (commit `d2f8554`)
- **Realized by:** `f-ingest-embedding-service`, `f-ingest-coordinator`,
  `f-ingest-staged-pipeline`, `f-ingest-store-seam`, `f-ingest-governance`
  (this ADR is the design gate those chunks implement)
- **Reasoning trace:** think:206/208 (open), think:259/260 (this ADR)

> This is an Architecture Decision Record, not an implementation. It locks the
> target design and decision criteria so the five implementation chunks can land
> independently without re-litigating the architecture. No production
> rearchitecture code ships with this document.

## Context — why today's ingestion is structurally unsafe

The current pipeline (`ministr-core/src/ingestion/pipeline.rs`) has four
structural problems, each of which has already produced a real defect:

1. **Synchronous GPU `embed()` runs on Tokio worker threads.** Embedding is a
   ~250 ms/batch GPU call invoked inline on the async runtime. When enough
   corpora index concurrently, every worker blocks inside `embed()` and the
   runtime starves → the daemon hang we band-aided with a global
   `INDEXING_SEMAPHORE(1)`, which now *over-serializes* (one corpus at a time).
2. **No global concurrency policy.** Each corpus watcher `tokio::spawn`s `run()`
   independently; the only governor is the semaphore=1 band-aid. There is no
   bounded worker pool, no backpressure, no priority, no dedup.
3. **One contended embedder.** A single shared embedder serializes on the GPU +
   a tokenizer `Mutex`; per-corpus batches are small and never merged.
4. **Split storage: SQLite content DB + a *separate* in-process HNSW file.**
   The HNSW has its own `persist`/`load`. A degenerate (zero) vector written to
   the HNSW could not be transactionally reconciled with SQLite → the
   zero-vector-poison bug (guarded in `fb3015a`) and the "fixed in code, stale on
   disk" class.

The god-function `ingest_paths_with_embeddings` (carrying
`#[allow(clippy::too_many_lines)]`) mixes discovery, parse, extract, embed,
persist, bridge-linking, orphan-GC, and stats in one body.

## Decision drivers (2026 research)

- **Embeddings want *dynamic* batching, not *continuous* batching.** Continuous
  / iteration-level batching (Anyssale "23×", BentoML, mbrenndoerfer 2026-01) is
  an *autoregressive-decode* optimization. An embedding is a *single encoder
  forward pass*, so the correct pattern is **dynamic batching**: queue requests,
  merge into one GPU batch up to a max size or a short max-latency timeout,
  scatter results back. Hugging Face **Text Embeddings Inference (TEI)** uses
  *token-based dynamic batching* as its recommended default
  (HF forums, 2025-12; TEI README). Scholarly 2025–2026 treats batch size as a
  throughput↔latency(↔energy) knob: Aalto, *Evaluating Dynamic Batching
  Strategies for Energy-Efficient Inference Serving* (2025); *ELTO:
  Energy-Latency Trade-off … with Dynamic Batching* (IEEE 2025); Zhao &
  Georgantas, *ML Inference Scheduling with Predictable Latency* (Middleware
  2025); *Coinf* (ACM TECS 2026). Multi-tenant GPU batch scheduling
  (IEEE TON-class work, 2025) is the literature form of "all corpora feed one
  queue."
- **Long-lived blocking work belongs on a dedicated OS thread, not
  `spawn_blocking`.** Authoritative: `tokio::task::spawn_blocking` docs —
  *"Use `spawn_blocking` for short-lived blocking operations; use dedicated
  threads for long-lived or persistent blocking workloads."* Reinforced by
  oneuptime (2026-01) — CPU work on the async runtime starves other tasks;
  offload to dedicated pools. (ministr already moved tree-sitter parse to a
  dedicated **rayon** pool in `332ece0`.)
- **Bounded worker pool + backpressure is the canonical scalable-Rust shape**
  (bounded channels + a `Semaphore` → backpressure, fault isolation, SRP;
  theopinionatedev / nashtech 2025-11).
- **`sqlite-vec` is brute-force-only KNN today.** Its ANN index is an *open*
  tracking issue (`asg017/sqlite-vec#25`); v0.1.x "will be brute-force search
  only, which slows down on large datasets (>1M w/ large dimensions)." Its real
  value is *radical simplicity/portability* — the whole index is one SQLite
  file. ANN ("a different-shape index — HNSW") is what you need at scale
  (Gothar, 2026-06). LanceDB is Rust-native columnar with IVF-PQ ANN +
  versioning, but a local-RAG benchmark (2026-05) notes it "chases perfect
  recall and pays for it on latency," and it is a heavier dependency.

## Decisions

### D1 — Embedding is a dedicated, dynamically-batched inference service

Introduce an `EmbeddingService` that **owns the model on its own OS thread**
(`std::thread`, long-lived — *not* `spawn_blocking`), fed by a **bounded `mpsc`
queue** of `EmbedRequest { texts, reply: oneshot }`. It **dynamically batches**:
drain the queue up to `MAX_BATCH` or a short `MAX_LATENCY` timeout, run **one**
GPU forward, scatter results back over the `oneshot` channels. **All corpora
feed this one queue**, so the GPU is never contended, the async runtime is never
blocked, and batches are maximized across corpora. It sits **behind the existing
`Embedder` trait** (DIP) — callers are unchanged; `CandleEmbedder` becomes the
model the service wraps. The degenerate/zero-vector guard (`fb3015a`) is applied
**before** results are returned. Ingest is throughput-oriented (not a latency
SLO), so bias the knob toward larger batch / longer timeout.

### D2 — A single Ingestion Coordinator owns concurrency policy

Replace per-corpus `spawn` + `semaphore=1` with one `IngestionCoordinator`
owning the **policy** (SRP): a job queue + a **bounded worker pool** (tunable
`N`, `1 < N < tokio_worker_count`) with backpressure via bounded channels + a
`Semaphore`. Because embedding is now the shared batched service (no GPU
contention), `N > 1` pipelines run concurrently for real throughput (one
corpus's parse/IO overlaps another's embed). The Coordinator also handles
**priority** (small code repos ahead of huge vendored trees), **dedup/coalescing**
of redundant reindex requests, **cancellation**, and a true
`Queued`/`Indexing`/`Idle` status surfaced from the scheduler.

### D3 — Staged pipes-and-filters behind stage traits

Decompose the god-function into explicit stages behind traits —
**Discover → Parse → Extract → Embed → Persist** — connected by bounded channels
(backpressure between every stage). Each stage is SRP, independently
unit-testable with fakes, and swappable (OCP/LSP): the Embed stage is the D1
service; the Persist stage is a D4 `CorpusStore` impl. The Coordinator composes
the stages. This removes the `clippy::too_many_lines` allow.

### D4 — `CorpusStore` trait; vectors live in the ACID store as source of truth

This is the decision that **structurally eliminates** the poison / stale-on-disk
bug class, and where this ADR refines the epic's original "sqlite-vec first."

Abstract storage behind a **`CorpusStore` trait (DIP)**. The load-bearing change
is **not** which ANN library we use — it is **making the ACID SQLite store the
single source of truth for vectors** (vectors commit *with* their metadata in one
transaction). Concretely:

- **Recommended first move:** vectors are persisted in the ACID store; the
  **HNSW becomes a *derived* in-memory ANN index, rebuilt from the store on
  load** and **never independently persisted**. There is no separate HNSW file to
  diverge, so the "fixed in code / stale on disk" + zero-vector-poison classes
  become *impossible* (the index is always reconstructable from the ACID truth,
  and the insert guard runs on the way in). This **keeps ANN speed** — which
  ministr needs for large and Atlas-scale corpora — while killing the bug class.
- **`sqlite-vec`** is a legitimate **alternative backend behind the same trait**
  for the small-corpus / maximum-simplicity case (one file, brute-force KNN —
  fine below ~10⁵ vectors), explicitly **not** the default, because brute-force
  regresses on the large/vendored/Atlas corpora ministr targets.
- **LanceDB** is the **columnar stretch** (vectors + metadata + bytes in one
  versioned store with IVF-PQ ANN) — evaluated, not adopted now.

The store-seam chunk spikes the source-of-truth flip first and **benchmarks**
recall / latency / ingest-throughput of derived-HNSW vs `sqlite-vec` before any
default changes.

### D5 — Resource governance + invariants as first-class stage guards

Cross-cutting safety for the new pipeline: **bounded memory** (cap in-flight
parse trees + embed-queue depth so a huge corpus can't OOM); **structured-
concurrency cancellation** (unregister/shutdown cleanly cancels a corpus
mid-pipeline; SQLite + vectors never disagree, extending the existing
`CancellationToken` rollback); **invariants as stage guards** (degenerate-vector
guard = Embed-stage invariant; no-partial-document = Persist-stage guarantee; a
CI guard detects a degenerate all-equal-distance index); and **per-stage metrics**
(throughput / latency / queue-depth) so the next bottleneck is measured, not
guessed.

## Build order (the five children, sequenced)

1. **`f-ingest-embedding-service`** (D1) — keystone; removes the runtime-starvation
   root cause and unblocks safe `N>1` concurrency.
2. **`f-ingest-staged-pipeline`** (D3) — gives the Coordinator composable stages
   and a home for the rayon-parse (`332ece0`) + the D1 embed stage.
3. **`f-ingest-coordinator`** (D2) — lifts inter-corpus concurrency above 1 with
   bounded backpressure; depends on D1 so concurrency is safe.
4. **`f-ingest-store-seam`** (D4) — `CorpusStore` trait + the vectors-as-ACID-truth
   spike + benchmarks; can proceed in parallel with 2–3.
5. **`f-ingest-governance`** (D5) — bounded memory + cancellation + invariants +
   metrics across the assembled pipeline.

`f-ingest-saturate-cpu-gpu` (the user's "saturate CPU+GPU" goal) is the
*observable outcome* of D1+D2+D3 landing, not a separate mechanism.

## Consequences

- **Positive:** the runtime can never starve (no blocking on async workers); many
  corpora index concurrently with bounded memory and never hang; the
  poison/stale storage bug class is structurally impossible; each stage is
  testable and swappable; the GPU is saturated by cross-corpus dynamic batches.
- **Costs/risks:** a dedicated embedding thread + queue is new machinery (mitigated
  by hiding it behind the existing `Embedder` trait); the storage source-of-truth
  flip touches persistence (mitigated by the trait + a benchmarked spike before any
  default change); `sqlite-vec`'s brute-force ceiling is why it is *not* the default.
- **Non-goals (this ADR):** changing the embedding *model*; a wire-protocol
  embedding server (the service is in-process); adopting LanceDB now.

## References (2026-focused)

- Hugging Face Text Embeddings Inference — token-based dynamic batching as the
  recommended default (HF forums, 2025-12; TEI README).
- towardsai, *How Modern Inference Servers Supercharge GPU Throughput with
  Batching* (2025-08); BentoML *Static/dynamic/continuous batching*; Baseten,
  *Continuous vs dynamic batching* (2024).
- Aalto, *Evaluating Dynamic Batching Strategies for Energy-Efficient Inference
  Serving* (2025); *ELTO: Energy-Latency Trade-off … Dynamic Batching* (IEEE
  2025); Zhao & Georgantas, *ML Inference Scheduling with Predictable Latency*
  (Middleware 2025); *Coinf* (ACM TECS 2026).
- `tokio::task::spawn_blocking` docs — short-lived vs dedicated-thread guidance;
  oneuptime, *Worker Threads in Rust for CPU-Intensive Tasks* (2026-01).
- theopinionatedev / nashtech (2025-11) — bounded channels + semaphore
  backpressure in scalable Rust.
- `asg017/sqlite-vec#25` (ANN tracking issue; brute-force-only); Gothar, *The SQL
  Blind Spot in 2026* (2026-06); local-RAG vector-DB benchmark (2026-05);
  firecrawl *Best Vector Databases in 2026*.
