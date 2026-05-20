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

- [ ] **Chunk 1 — RemoteEmbedder trait impl + Azure OpenAI client**
  - New `ministr-core::embedding::RemoteEmbedder` (MIT) — reqwest-based, batches into the API's max request size.
  - New `ministr-cloud::OpenAiEmbedder` (proprietary) — managed-identity bearer token from `DefaultAzureCredential`; resource = `https://cognitiveservices.azure.com/.default`; endpoint = `https://<resource>.openai.azure.com/openai/deployments/<deployment>/embeddings?api-version=2024-10-21`.
  - Behind a feature flag / env var: `MINISTR_EMBEDDER_KIND=local|openai`.
  - Unit tests against an axum mock (same pattern as `AcaJobStartTrigger`'s tests).
  - **Verify**: `cargo test`, mock round-trips, model dimension matches HNSW config.

- [ ] **Chunk 2 — WorkerLoop in the serve binary**
  - `ministr-cli::cmd_serve_http` spawns a background tokio task that polls `JobQueue::claim_next` every N seconds (configurable; default 5s — same as PHASE3 cron).
  - On claim: run `run_corpus_ingestion` exactly like `cmd_indexer_worker` does today, then `queue.finish`.
  - Concurrency cap: one in-flight job per replica. Backlog accumulates in `indexer_jobs.pending`.
  - The 500ms reporter from `cmd_indexer_worker` moves here verbatim.
  - **Verify**: integration test against a real Postgres + an in-process WorkerLoop drains a fake job.

- [ ] **Chunk 3 — Delete ACA Jobs from Pulumi + delete `job_start.rs`**
  - `deploy/azure/lib/job.ts` deleted.
  - `deploy/azure/lib/job-start-role.ts` deleted.
  - `deploy/azure/index.ts` drops `createIndexerJob` + `grantJobsOperator`.
  - `deploy/azure/lib/app.ts` drops the three `MINISTR_ACA_*` env vars.
  - `ministr-cloud/src/job_start.rs` deleted (PHASE5 chunk 1 + hotfix both revert).
  - `ministr-cloud/src/index_job_sink.rs` drops the `with_start_trigger` builder (no more fan-out site).
  - `ministr-cli/src/commands.rs` drops the `AcaJobStartTrigger` wiring block + `cmd_indexer_worker` function entirely.
  - **Verify**: workspace tests + clippy. The deleted-code path is the cleanest "did we miss a reference" signal.

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
