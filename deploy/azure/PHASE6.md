# Cloud Phase 6 — architecture reset: drop ACA Jobs, drop local embedding

> **Approved 2026-05-20.** User decisions locked in:
> - Managed embedder = **Azure OpenAI `text-embedding-3-small`** (not Voyage)
> - Local CLI = **unchanged** (fastembed stays for `ministr index` on the user's box)
> - Cost ceiling = **<$200/mo** (a guard-rail, not an optimization target)
>
> Replaces the PHASE5-chunk-3 follow-up direction.

## Why we're here

The current cloud architecture has churned for three phases without converging:

| Phase | Trigger | Symptom |
|---|---|---|
| PHASE3 | KEDA cron — every minute poll | $15/mo of empty-tick Postgres queries; deferred ARM RBAC for "one role" |
| PHASE4 | KEDA event scaler — every 5s Postgres poll | Marketing-event-driven only; replica startup cold-start dominates latency |
| PHASE5 | ARM `jobs/start` direct trigger from serve pod | ACA's non-VMSS IMDS broke it; hotfix landed; live demo OOM-kills mid-embedding |

The live demo from c00ad95 showed:

```
[mem] before embedder.embed()  rss=125 MB
[mem] after  embedder.embed()  rss=3762 MB  delta=+3637 MB
embedding progress chunk=3 of=13 embedded=384
[OOM-killed by ACA, replica gone, KEDA picks it up, also dies]
```

**Two distinct architectural problems are surfacing in the same failure:**

1. **ACA Jobs are the wrong primitive for our workload.** They are designed for short-lived, run-to-completion tasks (per Microsoft Learn). Each execution pays image-pull + replica-startup + model-load cost. We've stacked three different trigger mechanisms (cron, KEDA-poll, ARM) onto them because none is actually event-driven.
2. **Local ONNX embedding is too memory-hungry for our pod size.** First `.embed()` call allocates 3.6 GB — model + activation + ort's per-thread workspaces. We're hitting the 4 GiB ceiling on the first batch and OOM-killing during the first few chunks. The on-disk model is <80 MB; the runtime amplification is the problem.

Neither is fixable by tweaking the current layout. We need a reset.

## What's NOT broken (preserves the work-to-date)

- `ministr-core`'s ingestion pipeline — producer/consumer split, streaming HNSW persist (PHASE5 chunk 2), stat-merkle short-circuit. **Keep all of this.**
- The Postgres `indexer_jobs` table — the queue surface (FOR UPDATE SKIP LOCKED, claim_next, update_progress, finish). **Keep.**
- The PHASE5 chunk 3 wire shape — embeddings_done flowing to the SSE. **Keep.**
- Blob persistence (CorpusBlobStore, bundle round-trip). **Keep.**
- Postgres-backed corpus registry (PHASE3 chunk 1). **Keep.**
- The OAuth + tenancy + billing surfaces. **Keep.**
- The 1965 passing tests. **Keep.**

The trash pile is **Pulumi + the worker binary trigger path + the local embedder dependency on the cloud worker**. That's it.

## Proposed shape

### One Container App, two roles, no Jobs

```
                      ┌────────────────────────────────────┐
                      │  Postgres (Flexible Server)        │
                      │  - oauth_*, users, orgs            │
                      │  - cloud_corpora                   │
                      │  - indexer_jobs (queue)            │
                      └────────────────────────────────────┘
                                ▲          ▲
                                │          │
                                │INSERT    │claim_next + update_progress
                                │          │
   ┌────────────────────────────┴──────────┴────────────────────┐
   │  ministrv2-app   (Container App, minReplicas=1, max=N)     │
   │                                                            │
   │  ┌─────────────────┐         ┌──────────────────────────┐ │
   │  │  HTTP / MCP     │         │  WorkerLoop (background)  │ │
   │  │  - /api/v1/*    │ enqueue │  - polls indexer_jobs     │ │
   │  │  - /mcp         │────────▶│  - drains FOR UPDATE      │ │
   │  │  - /healthz     │         │    SKIP LOCKED            │ │
   │  │  - /webhooks    │         │  - runs ingestion in      │ │
   │  └─────────────────┘         │    tokio task             │ │
   │                              │  - max-in-flight=1 per    │ │
   │                              │    replica                │ │
   │                              └──────────────────────────┘ │
   │                                                            │
   │  Memory budget: 2 GiB / 1 vCPU (was 4 GiB / 2 vCPU on     │
   │  the Job). Embedder calls a network API, not local ONNX.  │
   └────────────────────────────────────────────────────────────┘
                                ▲
                                │ POST /v1/embeddings
                                │
                ┌───────────────┴──────────────┐
                │  Managed Embedding API       │
                │  Azure OpenAI text-emb-3-sm  │
                │  OR Voyage voyage-3-lite     │
                │  ($0.02 per 1M tokens)       │
                └──────────────────────────────┘
```

What this eliminates:

- **The indexer Job resource entirely** (`deploy/azure/lib/job.ts`).
- **KEDA postgres scaler** — gone with the Job.
- **ARM `jobs/start` trigger** — `ministr-cloud::AcaJobStartTrigger` becomes dead code; revert PHASE5 chunk 1 entirely.
- **`Container Apps Jobs Operator` role** — gone.
- **The IDENTITY_ENDPOINT IMDS dance** — gone with the ARM call (the rest of the pod still uses ManagedIdentityCredential for blob/postgres, which works fine on ACA).
- **The 3.6 GB ONNX activation memory** — embedder is now a `reqwest::Client`.
- **"No replicas found for execution"** failures — there are no executions, just a long-lived replica.
- **Image-pull cost per ingest** — the replica stays warm.
- **First-inference cost per ingest** — model is "loaded" exactly never (it's at Azure OpenAI's side).

What this preserves:

- Multi-replica scaling for throughput (just bump `maxReplicas`).
- Backpressure (worker only claims when it has capacity).
- Postgres queue durability across pod recycle.
- Blob durability for bundles.
- Pull-model worker (no inbound triggers needed).
- The whole self-hosted path (local fastembed stays for `ministr index` on user's box).

### What changes in the code

**`ministr-core`**:
- Add a `RemoteEmbedder` impl of the existing `Embedder` trait. POSTs to a configured endpoint with bearer token. ~150 LoC.

**`ministr-cloud`**:
- Add `OpenAiEmbedder` (managed Azure OpenAI client). Bearer token from MI or static key.
- Optionally: `VoyageEmbedder` (separate provider). Same trait.
- **Delete** `job_start.rs` entirely (PHASE5 chunk 1 revert). Keep `index_job_sink.rs` and its tests.

**`ministr-cli`**:
- New `cmd_serve_with_worker` (or extend `cmd_serve_http`): spawn a `WorkerLoop` background task that polls `indexer_jobs` via `JobQueue::claim_next`, runs ingestion in-process, updates progress, calls finish.
- The reporter we already have for `cmd_indexer_worker` moves into the worker task.
- Drop `cmd_indexer_worker` entirely.

**`deploy/azure/`**:
- Delete `lib/job.ts` + `lib/job-start-role.ts`.
- Drop the `createIndexerJob` call from `index.ts`.
- Drop the `MINISTR_ACA_*` env vars from `lib/app.ts`.
- Set `ministrv2-app` `maxReplicas: 3` (or whatever; tunable).
- Optional: lower default `appMemory` from 1 GiB to 2 GiB (the worker now runs in-process — needs SQLite + HNSW + tokenization buffers, but not the ONNX model).

**`Dockerfile`**:
- Could remove the fastembed model download step from the cloud image (it's still useful for the local CLI). Cleaner to keep one image; just don't load the model when `MINISTR_EMBEDDER_KIND=remote`.

### Cost & dimension trade-offs

Current state (PHASE5, even when it works): ~$10–15/mo Azure baseline + intermittent ACA Job compute + KEDA Postgres polls.

Post-refactor:

- **Container App stays at 1 replica minimum** (already $0 when not serving HTTP). Same baseline.
- **Embedding cost**: at $0.02/1M tokens, an anyhow-sized ingest (~466K tokens) is **~$0.01**. A user indexing 10 medium repos/month is **~$0.10/user**. Below the Pro tier's $20/mo by 200×.
- **At Atlas scale** (~5K curated repos × weekly re-index): 5K × 466K tokens × 4 weeks = 9.3B tokens/month = **$186/mo**. Still substantially cheaper than the current spot-CPU plan in PHASE4.2's cost envelope ("250 spot-hours/wk ≈ ~$150–250/mo + storage").

The break-even point for self-hosted ONNX vs managed API is well above our current scale.

### Decisions (locked)

| # | Decision | Why |
|---|---|---|
| 1 | **Azure OpenAI `text-embedding-3-small`** for the cloud embedder | Already in the subscription; no second vendor; $0.02/1M tokens; well-trodden REST surface. If we ever want Voyage as a second provider, the `RemoteEmbedder` trait makes it a one-file add. |
| 2 | **Local CLI stays unchanged** (fastembed) | Local `ministr index` runs on the user's box where memory is plentiful and offline-capable is a feature. Only the cloud worker swaps to remote. |
| 3 | **Cost ceiling: under $200/mo** | Guard-rail, not optimization target. At realistic Pro-tier usage we're orders of magnitude below this. At Atlas weekly-re-index scale we're inside the ceiling. |

## Chunks

Atomic, one per `/roadmap PHASE6` invocation:

- [x] **Chunk 1 — `OpenAiEmbedder` against Azure OpenAI** *(code + tests landed; wiring lands with chunk 2's `WorkerLoop`)*
  - **Decision revision**: dropped the proposed `RemoteEmbedder`
    intermediary trait — the existing `ministr-core::embedding::Embedder`
    surface is already the right contract. The concrete type lives in
    `ministr-cloud` (proprietary) and impls `Embedder` directly.
  - [x] `ministr-cloud::OpenAiEmbedder` ships with two auth paths:
    `OpenAiAuth::ApiKey` (read from `MINISTR_AZURE_OPENAI_API_KEY`,
    sets the `api-key` header) and `OpenAiAuth::ManagedIdentity` (reads
    `IDENTITY_ENDPOINT` + `IDENTITY_HEADER`, mints a bearer for
    `https://cognitiveservices.azure.com` via the same ACA IMDS shape
    PHASE5's hotfix uses, then `Authorization: Bearer …`). MI tokens
    cached with proactive evict; same pattern as `GitHubAppClient`.
  - [x] `OpenAiAuth::from_env()` auto-selects (`ApiKey` wins if set,
    falls back to MI). `OpenAiConfig::from_env()` returns `None` when
    any of (endpoint, deployment, auth) are missing so the caller can
    fall back to local fastembed cleanly.
  - [x] Request shape: `POST {endpoint}/openai/deployments/{deployment}/embeddings?api-version=2024-10-21`
    with body `{ "input": [...], "dimensions": 384 }`. Default
    dimension is 384 (`DEFAULT_DIMENSIONS`) to keep HNSW indexes
    cross-compatible with the local `all-MiniLM-L6-v2*` family;
    operators wanting full 1536-dim quality build with
    `with_dimensions(1536)` and accept incompatible indexes.
  - [x] Sync-over-async: uses `reqwest::blocking::Client` (workspace
    feature `blocking` enabled). The worker is single-concurrent per
    replica so blocking the tokio thread for the ~500ms–2s request is
    acceptable; chunk 2's `WorkerLoop` calls `embed()` from inside a
    `spawn_blocking` for hygiene.
  - [x] Response handling: sorts response rows by `index` (Azure spec
    guarantee, defended), enforces returned-batch-size matches
    requested-batch-size, enforces every vector's dimension matches
    the configured `dimensions`. All three checks surface as
    `IndexError::EmbeddingFailed { reason }` with diagnostic detail.
  - [x] 9 round-trip tests against an axum mock (mirrors
    `AcaJobStartTrigger::tests` pattern): API-key path, MI path, MI
    token cache survives across batches, 4xx body surfaces with status
    code + message excerpt, batch-size mismatch caught, empty input is
    a no-op, dimension reporting, dyn-trait dispatch.
  - **Honest finding**: `reqwest::blocking::Client` can't be dropped
    from inside an outer tokio runtime — its internal runtime panics.
    Tests build the embedder inside `spawn_blocking` so the full
    lifecycle stays on a blocking thread. Documented in the test
    module's preamble; the production `WorkerLoop` follows the same
    pattern.
  - [x] Workspace verification clean: 1977 lib tests passing (+9
    new); workspace clippy pedantic clean.

- [x] **Chunk 2 — WorkerLoop in the serve binary** *(code + 4 in-memory drive tests landed; live `azure-demo` confirmation pending operator run)*
  - [x] New `ministr-cli/src/worker.rs` module: `WorkerLoop` + `JobRunner` trait + `IngestionRunner` production impl. Lives in `ministr-cli` (MIT) because the loop is self-hosted-compatible; the cloud crate just contributes concrete impls.
  - [x] `WorkerLoop` polls `JobQueueBackend::claim_next` every 5s (default; `with_poll_interval` test override). Cancellation via shared `CancellationToken`. Strictly serial — one job in-flight per replica; backlog drains as jobs complete; scale by adding Container App replicas.
  - [x] `IngestionRunner` lifts the body of the (soon-deleted) `cmd_indexer_worker` verbatim: builds per-job `InfrastructureContext`, runs `run_corpus_ingestion` with `persist_every=4`, uploads bundle to blob, finishes the row. The 500ms reporter (incl PHASE5 chunk 3 embedding fields) moves here verbatim.
  - [x] **Embedder selector** lands in `infra::init_infrastructure`: when `MINISTR_EMBEDDER_KIND=openai`, build `OpenAiEmbedder` and SKIP the local fastembed model load entirely — no ONNX init, no model download. Hard-error (not silent fallback) when the env var is set but `OpenAiConfig::from_env()` doesn't resolve. Local CLI unchanged.
  - [x] **Chunk 1 embedder revision**: refactored `OpenAiEmbedder` from `reqwest::blocking::Client` to async `reqwest::Client` + sync-bridge via `tokio::task::block_in_place` + `Handle::block_on`. Blocking client panics on drop inside a tokio runtime — and the serve binary holds the embedder Arc for the process lifetime, so the drop happens at `#[tokio::main]` shutdown inside the runtime. New `OpenAiEmbedder::embed_async` is the public async surface; the sync trait method is a thin bridge.
  - [x] **Wired in `cmd_serve_http`**: opens `PostgresJobQueue`, wraps in `JobQueueBackend::Postgres`, builds the `IngestionRunner` with the existing blob backend, spawns the `WorkerLoop` on a detached tokio task. Runs alongside the legacy ACA Job during the chunk-2-to-chunk-3 transition; both compete via `FOR UPDATE SKIP LOCKED` so no correctness issue.
  - [x] Tests: 4 new `worker::tests` against `JobQueueBackend::InMemory`: cancel-before-claim, claim+run+finish, runner-error→Failed, drains-3-queued-in-sequence. Embedder tests grew to 11 (was 9) with `embed_sync_bridge_works` and `embed_sync_bridge_errors_outside_runtime`.
  - [x] Workspace verification clean: **1983** total tests passing (1979 lib + 4 worker bin); workspace clippy pedantic clean.
  - **Honest finding**: the chunk-1 embedder refactor was load-bearing. Chunk 1 shipped with `reqwest::blocking::Client`; tests passed only because they explicitly dropped the embedder inside `spawn_blocking`. Production drop in `cmd_serve_http` would panic at shutdown. Caught while exploring `init_infrastructure` for chunk 2.

- [x] **Chunk 3 — Delete ACA Jobs from Pulumi + delete `job_start.rs`**
  - [x] `deploy/azure/lib/job.ts` deleted.
  - [x] `deploy/azure/lib/job-start-role.ts` deleted.
  - [x] `deploy/azure/index.ts` drops `createIndexerJob`, `grantJobsOperator`, `indexer-blob-rw` role, `indexerJobName` output, `authorization` import. `jobCpu`/`jobMemory` Pulumi config keys are now orphan but harmless — operator can `pulumi config rm` if they want.
  - [x] `deploy/azure/lib/app.ts` drops the three `MINISTR_ACA_*` env-var inputs + the threading at the bottom of `baseEnv`.
  - [x] `deploy/docker-entrypoint.sh` drops the `indexer-worker` mode (was the ACA Job entrypoint); accepted modes are now just `serve` and `index`.
  - [x] `ministr-api/src/job_start_trigger.rs` deleted (the MIT trait went with the only impl).
  - [x] `ministr-cloud/src/job_start.rs` deleted (PHASE5 chunk 1 + the ACA IMDS hotfix both revert in one motion).
  - [x] `ministr-cloud/src/index_job_sink.rs` drops the `start_trigger` field + `with_start_trigger` builder + the post-commit `tokio::spawn` fan-out call. `create_pending` is back to just committing the txn.
  - [x] `ministr-cli/src/commands.rs` drops the three `MINISTR_ACA_*` fields from `CloudEnv` + their `read_cloud_env` lines + the entire `AcaJobStartTrigger` build/match block in `cmd_serve_http` + the whole `cmd_indexer_worker` function (~245 lines).
  - [x] `ministr-cli/src/main.rs` drops the `IndexerWorker` Command variant + its dispatch arm.
  - [x] Re-exports cleaned: `ministr-api/src/lib.rs` no longer surfaces `JobStartError/Future/Trigger`; `ministr-cloud/src/lib.rs` no longer surfaces `AcaJobStart*` or `ImdsAuth`.
  - **Verify**: `cargo build --workspace` clean; workspace tests **1973 passing** (1969 lib + 4 worker bin; the 10 `job_start::tests` are gone with the file); workspace clippy pedantic clean; `cd deploy/azure && npx tsc --noEmit` clean.
  - **Operator action remaining**: `pulumi up` will see the indexer Job + two role assignments + four env vars as deletions — confirm the diff looks right before applying. The first `pulumi up` after this chunk is the actual demolition. Chunk 4 wraps that with a fresh `azure-demo`.

- [ ] **Chunk 4 — `pulumi up` + first live azure-demo on new arch**
  - First deploy with `MINISTR_EMBEDDER_KIND=openai` and a deployment name pinned via Pulumi config.
  - Provision the Azure OpenAI resource if not present (one new Pulumi module `lib/openai.ts`).
  - Demo expectation: clone anyhow → SSE shows embeddings_done climbing in real time → completes in <5 min → blob upload succeeds.
  - **Verify**: operator runs `just azure-demo`, observes a healthy completion, files an issue here if not.

## What's NOT in this phase

- Replacing fastembed locally. The CLI still ships ONNX.
- Worker-pool tuning beyond `maxReplicas: 3` and `concurrency_per_replica: 1`. Tune once we have real traffic.
- Voyage as a second managed provider. Add later if quality on code retrieval warrants.
- Multi-region. Single region (eastus) stays.
- INT8 fastembed on the cloud. Moot; cloud doesn't load fastembed any more.

## What this gives up

Honest list:

1. **Per-ingest cost is now a real line item.** $0.01 per medium repo is small but non-zero.
2. **Embedding quality is tied to OpenAI's release cadence.** If they sunset `text-embedding-3-small`, we migrate. Mitigation: the `RemoteEmbedder` trait makes swapping providers a one-file change.
3. **Network dependency on Azure OpenAI being up.** ACA-to-Azure-OpenAI is same-region; effectively no real WAN dependency, but it IS another service.
4. **Higher per-replica memory floor than the previous serve pod** because the worker runs in-process. Worth ~500 MB extra. Still well under 2 GiB.

These are all known and bounded. None reproduces the PHASE5 OOM.

## Verify

Same recipe as PHASE5:
- Rust: `cargo build --workspace && cargo test --workspace --lib && cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`
- Pulumi: `cd deploy/azure && npx tsc --noEmit`
- Cloud smoke: `just azure-demo`

## Why this is PHASE6 and not patches on PHASE5

PHASE5 added three new artefacts (ARM trigger, persist gate, SSE wire shape). Two of them survive (persist gate is unchanged; SSE wire shape is unchanged). The third — ARM trigger + job-start role + IDENTITY_ENDPOINT plumbing — is being thrown away wholesale. Clean phase boundary keeps the postmortem honest.

The thing PHASE6 also acknowledges: the user-visible churn through PHASE3→4→5 was symptom-driven, not root-cause-driven. Each phase tried to fix the visible symptom of the previous (cron costs → fix with KEDA → KEDA isn't event-driven → fix with ARM → ARM is fragile on ACA → OOM). The root cause was: **ACA Jobs are not the right primitive for embedding-heavy workloads on small pods**, full stop. PHASE6 names that and changes the primitive.
