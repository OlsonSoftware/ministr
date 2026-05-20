# Cloud Phase 4 — event-driven worker + streaming ingestion

PHASE3 (May 2026) split the serve and worker pods, made the corpus
registry durable in Postgres, and put a scheduled-cron worker behind
an `indexer_jobs` queue. It shipped end-to-end in six chunks. The
post-deploy smoke surfaced four bugs and one architectural truth that
together justify a new phase rather than patches on PHASE3.

## What PHASE3 actually shipped (and what it taught us)

| Chunk | Status | Cost it imposed | Lesson |
|---|---|---|---|
| 1: `cloud_corpora` Postgres registry | ✅ working | none | Right move; durable across pod recycle. |
| 2: `JobTrigger::Tenant` variant | ✅ working | none | Right move; serde-stable on the wire. |
| 3: `cmd_indexer_worker` entrypoint | ✅ working | none structural | Right move; single-shot semantics are clean. |
| 4: serve-pod enqueue + queue-backed SSE | ✅ working | progress events go silent during ingest — fixed mid-PHASE3 (Fix B) | The chunk-3 worker emitted no progress, exposed by the chunk-4 SSE. Coupled changes need coupled validation. |
| 5: on-demand bundle restore | ✅ working | none | Right move; replaces PHASE2's boot-time bulk download. |
| 6: ACA Job cron-poll trigger | ✅ working but wasteful | ~$15/mo on empty ticks, 0-60s trigger latency, no event-driven scaling | **The substrate is right; the trigger model was wrong.** |

Mid-PHASE3 bug fixes that landed on the live cluster:

- **Fix A** — empty-corpus HNSW persist skipped (the demo's parent-corpus placeholder ingests 0 files; persist-with-0-points crashes the worker).
- **Fix B** — worker emits `queue.update_progress` every 500ms during ingestion so the serve pod's queue-backed SSE shows real per-file progress instead of stuck `0/0`.
- **Fix C** — empty-corpus bundle upload skipped (`bundle::export_bundle` walks a path that resolves to `/` on empty `corpus_roots`).
- **Fix E** — `MINISTR_PREFER_QUANTIZED=1` on the indexer Job. Saved ~60 MB at boot but did not solve OOM — quantization affects weights, not activations.
- **Robust RBAC** — `lib/role-assignment.ts` switched to deterministic UUID-v5 role-assignment GUIDs + `ignoreChanges: ["scope", "principalType", "roleAssignmentName"]` + `replaceOnChanges: ["principalId"]`. Stops the orphan-on-replace accumulation that left phantom principals (`815585fd…`, `76231d1a…`) on the storage account.

What didn't get fixed in PHASE3 and is the reason PHASE4 exists:

1. **`Bug D` — opaque blob upload failure** (RBAC). Resolved operationally via `just azure-rbac-reconcile` + the deterministic-GUID source change, but the failure mode itself reveals that **the worker has no progress reporting before/around the upload step** and no claim reclaim, so the first crash leaves a `running` row permanently stuck.
2. **`Bug E` — OOM at chunk 10/13 on anyhow (44 files, ~1500 vectors)**. Memory grows ~600 MB per file (activation arenas + AST + claim graph + ONNX runtime). 8 GiB is insufficient, 12 GiB is a workaround, 16 GiB postpones the inevitable on a larger repo. **The pipeline is monolithic** — it loads the entire corpus into memory before persisting + uploading. Quantizing weights, bigger pods, smaller batches each shave a constant; the real fix is streaming.

## Goal

Two structural changes — one substrate, one pipeline — that make PHASE3's serve/worker split production-ready:

1. **Event-driven worker** — the ACA indexer Job listens on the Postgres `indexer_jobs` queue via KEDA's `postgresql` scaler. Replicas spin up *only* when there's pending work. Latency improves from cron tick (0-60s) to KEDA poll (~5-30s); empty-tick cost drops to ≈$0/mo. Adds `claimed_at` reclaim so a crashed worker's row doesn't sit in `running` forever.
2. **Streaming ingestion** — `ministr_core::ingestion::IngestionPipeline` switches from corpus-wide load-then-persist to per-file-batch embed-and-persist. Memory becomes O(batch_size) instead of O(corpus_size). Worker can shrink to 4 vCPU / 4 GiB; large corpora become feasible regardless of size.

A speculative PHASE5 (managed embedding API) is outlined at the end as a strategic option, not a planned chunk.

## Architecture

```
                ┌────────────────────────────────┐
                │  Postgres (existing)           │
                │   cloud_corpora                │
   POST /api/v1/corpora                          │
        │       │   indexer_jobs                 │
        ▼       │     id, corpus_id, status,     │
  ┌─────────────┤     trigger, claimed_at,       │
  │  Serve pod  │     progress_blob, ...         │
  │  (0.5 vCPU) ├──── INSERT ────────────────────┤
  │             │                                │
  │  read-only  │     ▲   FOR UPDATE SKIP LOCKED │
  │  queries    │     │   ORDER BY priority,     │
  │  + SSE      │     │           created_at     │
  └──────┬──────┘     │                          │
         │            │                          │
         │ download   │   KEDA postgresql scaler │
         ▼            │   queries pending count  │
  ┌──────────────┐    │   every ~5s              │
  │  Azure Blob  │    │                          │
  │ ministr-     │    │                          │
  │ corpora      │    └──────┬───────────────────┘
  └──────┬───────┘           │
         │                   ▼
         │           ┌─────────────────────────┐
         │ upload    │  Indexer Job (Event)    │
         └───────────┤  4 vCPU / 4 GiB         │
                     │  scale 0→N on queue     │
                     │                         │
                     │  1. claim_next          │
                     │  2. for file in source: │
                     │       parse + chunk     │
                     │       embed batch       │
                     │       persist batch     │   ← streaming
                     │       update_progress   │
                     │       free memory       │
                     │  3. finalise bundle     │
                     │  4. upload bundle       │
                     │  5. finish(Completed)   │
                     └─────────────────────────┘
```

Concretely changes from PHASE3:

- `lib/job.ts`: `triggerType: "Schedule"` → `"Event"` with `scaleRules` pointing at the queue. Default 5s polling, scale-to-zero, max replicas 1 (single-tenant cloud today).
- `migrations/0004_indexer_jobs_claimed_at.sql`: `ALTER TABLE indexer_jobs ADD COLUMN claimed_at TIMESTAMPTZ` + index.
- `ministr-mcp/src/admin/jobs/postgres.rs::claim_next`: sets `claimed_at = now()` on the transition to `running`. Also exposes a new `reclaim_orphans(timeout)` method that UPDATEs back to `pending` rows whose `claimed_at` is older than `replicaTimeout`.
- `ministr-cli/src/commands.rs::cmd_indexer_worker`: calls `reclaim_orphans` once at startup before `claim_next`.
- `ministr-core/src/ingestion/pipeline.rs`: refactored — `ingest_paths_streaming` that takes a file iterator and writes to storage + HNSW after each batch (default 4 files). Old `ingest_paths_with_embeddings` deprecated for cloud, kept for the local one-shot `ministr index`.
- `deploy/azure/Pulumi.prod.yaml`: `jobCpu: 4`, `jobMemory: 4Gi` (down from 12Gi after PHASE3's bump).

## Chunks (atomic, one per `/roadmap` invocation)

- [x] **Chunk 1 — KEDA event-driven trigger (Pulumi only)** — `lib/job.ts` swapped Schedule for `triggerType: "Event"` with `eventTriggerConfig.scale.rules` carrying a `postgresql` KEDA scaler against `indexer_jobs` (status='pending' count, polling 5s, min/max 0/1). Auth via the existing `pg-url` secret on `triggerParameter: "connection"`. `npx tsc --noEmit` clean. Correction vs the doc: actual KEDA metadata key is `targetQueryValue`, not `targetValue`. Pulumi.prod.yaml left untouched in this chunk — chunk 5 right-sizes after streaming lands.
  - Cloud smoke (operator-run after `pulumi up`): `just azure-jobs` should show zero executions when the queue is empty; a `POST /api/v1/corpora` should produce one execution within ~10s of insert. *Pending real-cloud verification.*

- [ ] **Chunk 2 — `claimed_at` + reclaim sweeper**
  - Migration `0004_indexer_jobs_claimed_at.sql` adds the column + an index `(status, claimed_at)`.
  - `PostgresJobQueue::claim_next` sets `claimed_at = now()` on the running transition.
  - New `PostgresJobQueue::reclaim_orphans(timeout_secs: i64) -> JobResult<usize>` that flips `running` rows with `claimed_at < now() - INTERVAL '<timeout>s'` back to `pending` (and clears `claimed_at`). Returns count reclaimed.
  - `cmd_indexer_worker`: call `reclaim_orphans(3600)` (matches ACA `replicaTimeout`) once at boot, before `claim_next`. Log the count.
  - Tests against `MINISTR_TEST_PG_URL`: simulate a crash by manually flipping a row to `running` with stale `claimed_at`, run reclaim, verify it's `pending` again.

- [ ] **Chunk 3 — Streaming ingestion design + migration plan**
  - This chunk is **design + scaffolding only**, not the big refactor. Splits the existing `IngestionPipeline::ingest_paths_with_embeddings` into composable phases (discover, parse, embed, persist) without changing the contract. Adds an internal `BatchIngestionConfig { batch_size: usize, persist_every: usize }` that's currently set to "everything at end" to preserve behaviour.
  - Writes the design doc inline: how `ministr-core::index::HnswIndex` handles incremental adds vs full builds (currently full-build via `with_config`); whether we need an `append_batch + persist_incremental` API.
  - No behavioural change. Just the refactor surface so chunk 4 can plug in.

- [ ] **Chunk 4 — Streaming ingestion implementation**
  - The actual swap: per-batch parse → embed → persist → free. Default `batch_size: 4`.
  - HNSW: `index.add_batch(...) + index.persist()` after each batch (HNSW supports incremental adds — verify in chunk 3).
  - Storage (SQLite): commits per batch instead of one big transaction.
  - Bundle export still runs at the end (corpus_roots + manifest), so a one-time spike there. Measure peak rss; should be ~2 GiB on anyhow.
  - Verified by: anyhow ingest completes on a 4 GiB / 4 vCPU job. `mem_profile` peak well under 4 GiB.

- [ ] **Chunk 5 — Right-size worker post-streaming**
  - `Pulumi.prod.yaml`: `jobMemory: 4Gi`, `jobCpu: 2`. (CPU drop only if ingestion-pipeline benchmarks show 4 vCPU was over-provisioned.)
  - Cost: from ~$4/h-when-running at 4 vCPU / 12 GiB down to ~$2/h at 2 vCPU / 4 GiB.
  - Operator-side smoke: `just phase3-smoke` (rename to `just phase4-smoke`?) ends-to-end on the new spec, including the pod-restart-then-query step.

- [ ] **Chunk 6 — Observability + ops polish**
  - `just azure-orphans` recipe: lists `cloud_corpora` rows whose `indexer_jobs` history has no `Completed` entry, OR role-assignments on the storage account whose principals don't resolve. Helps catch state drift before it bites.
  - SSE: emit one final event with terminal `status` + summary stats before close, so the demo client doesn't need to special-case the EOF.
  - `just azure-cost` recipe: queries Log Analytics for indexer-job execution-seconds per day and estimates monthly cost. Used to validate chunk 1's cost claim.

## Open questions

- ~~**KEDA `postgresql` scaler auth in ACA.**~~ *Resolved in chunk 1.* Use the standard ACA `JobScaleRule.auth` array mapping the `pg-url` secret onto `triggerParameter: "connection"` — the same TriggerAuthentication pattern KEDA uses upstream. No `connectionFromEnv` needed.
- **`HnswIndex` incremental add semantics.** Our wrapper around `hnsw_rs` builds the index in one shot via `with_config`. Need to confirm `add_one`/`add_batch` work incrementally and that `persist` is safe to call mid-build (the tmp-rename trap from Fix A may recur if persist is called on an in-flight build).
- **Streaming + claim coherence.** If the worker streams progress and the next poll mid-ingest sees claimed_at is fresh (because the worker just updated it), reclaim shouldn't fire. That's the design, but worth a unit test where progress-update bumps claimed_at as a side effect.
- **PHASE5 trigger.** When (if ever) to flip to managed embedding API. Open until we have a sustained workload that pushes streaming past its limits.

## Discovered / backlog (inherited from PHASE3 + new)

- [ ] **Per-tenant blob prefix** — for multi-tenant pivot.
- [ ] **Worker concurrency > 1** — already supported by `FOR UPDATE SKIP LOCKED`; just needs `parallelism: 2+` in chunk 1's `eventTriggerConfig`. Defer until a single-replica's serial drain becomes a bottleneck.
- ~~**Azure REST trigger from serve pod (PHASE3 chunk 6 option B)**~~ — superseded by KEDA event trigger (chunk 1 shipped). Removable.
- [ ] **Disk eviction in the serve pod** — restored corpora pile up under `/data` until pod recycle. Will become a real issue once on-demand restore (PHASE3 chunk 5) sees regular traffic.
- [ ] **Promote `BlobError::is_not_found` to public** — the chunk-5 `BlobCorpusRestorer` string-shape probes for it; expose the typed helper.
- [ ] **github-app `installation_id` in `JobTrigger::Tenant`** — for clone-mode github-app cloning (PHASE3 chunk 4 returned 501 for this).
- [ ] **PHASE 5: managed embedding API** — `text-embedding-3-small` (1536-dim) via Azure OpenAI. Removes ONNX/fastembed from the worker entirely. Cost: ~$0.02/1M tokens (~$0.01 per full reindex of anyhow). Tradeoff: vendor lock-in + 384→1536 dimension change requires re-embedding existing corpora. Decision deferred; only worth shipping if streaming (chunk 4) hits a wall.

## Verify

- Rust: `cargo build --workspace && cargo test --workspace --lib && cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`
- Pulumi: `cd deploy/azure && npx tsc --noEmit`
- Cloud smoke per chunk 5 (after streaming lands).

## Recipes added by PHASE4

(Each chunk that ships an operator-facing capability adds a `just` recipe per the standing convention from PHASE3.)

- `just azure-jobs` — already shipped in PHASE3; remains the primary "what's the worker doing" view.
- `just azure-rbac-reconcile` — already shipped in PHASE3; one-time RBAC drift fix.
- `just azure-orphans` — new in chunk 6; lists drift between `cloud_corpora` and `indexer_jobs` history, and orphan role-assignments.
- `just azure-cost` — new in chunk 6; per-day indexer-job billable seconds from Log Analytics.

## Why this is PHASE4 and not PHASE3.5

PHASE3 shipped what it set out to ship. The serve/worker split is real and works. The follow-on changes here are not "finishing" PHASE3 — they're a separate architectural decision (event-driven vs polled; streaming vs monolithic) that PHASE3's design didn't preclude but also didn't require. Treating it as a new phase keeps each phase doc honest about what it actually delivered.
