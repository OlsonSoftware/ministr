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

- [x] **Chunk 2 — `claimed_at` + reclaim sweeper** — added a `claimed_at TIMESTAMPTZ` column + `(status, claimed_at)` index to `indexer_jobs` (via `ensure_schema`'s idempotent `ALTER TABLE … ADD COLUMN IF NOT EXISTS`, not a separate `migrations/0004_...` — that dir owns `cloud_corpora`, indexer_jobs is set up in-process). `PostgresJobQueue::claim_next` now stamps `claimed_at = NOW()`. New inherent method `PostgresJobQueue::reclaim_orphans(timeout_secs: i64) -> JobResult<usize>` uses `SELECT … FOR UPDATE SKIP LOCKED` then a per-row deserialise→mutate→UPDATE so the status flip lands in *both* the column and the JSON `data` blob (the `get()` path reads from the blob). `cmd_indexer_worker` calls `reclaim_orphans(3600)` once at boot, soft-failing on error. Integration test `reclaim_orphans_recovers_stale_running_rows` covers the happy path + the fresh-row-untouched path; gated on `MINISTR_TEST_PG_URL` like the rest of the postgres suite.
  - `cargo build --workspace`, `cargo test -p ministr-mcp --lib` (237 pass, 10 ignored — the postgres integration suite), `cargo clippy -p ministr-mcp -p ministr-cli --all-targets -- -D warnings -W clippy::pedantic` all clean.
  - Honest negative: the new reclaim test is in the ignored set; real-Postgres verification waits on CI or a manual `MINISTR_TEST_PG_URL` run.

- [x] **Chunk 3 — Streaming ingestion design + migration plan** — added `pub struct BatchIngestionConfig { batch_size, persist_every: Option<usize> }` to `ministr_core::ingestion` and plumbed it through `IngestionPipeline` (new private field + `with_batch_config()` builder). `Default` preserves PHASE3 behaviour (`persist_every: None`). The struct's rustdoc captures the four-phase model (discover/parse/embed/persist) and three findings worth flagging: (a) the pipeline is **already streaming** at the parse↔embed boundary via mpsc + `buffer_unordered`; (b) `HnswIndex::insert` is already incremental — `with_config` is one construction path, not the hot path; (c) `HnswIndex::persist` is atomic (tmp-rename + fsync per the existing `atomic HNSW persist with crash-recovery backup` change), so mid-run persist is safe today.
  - `cargo build --workspace`, `cargo clippy -p ministr-core --all-targets -- -D warnings -W clippy::pedantic`, `cargo test -p ministr-core --lib` (1495/1495 pass) all clean.
  - **Plan deviation for chunk 4:** the doc framed chunk 4 as "HNSW supports incremental adds — verify in chunk 3" + "storage commits per batch instead of one big transaction". Reality: HNSW is already incremental, and SQLite already commits per file (via `parse_and_store_file`). The OOM lever isn't there — it's the per-file *intermediate state* (parsed sections, claim graphs, ONNX activations) held in process memory until end-of-ingest. Chunk 4 should focus on (a) consuming `persist_every` to flush HNSW to disk periodically (cheap), and (b) explicitly freeing the producer's per-file intermediates after each embedding batch is sent.

- [x] **Chunk 4 — Streaming ingestion implementation** — (1) consumed `BatchIngestionConfig::persist_every` in `run_producer_consumer`: each `FileResult::Indexed` increments the file counter; when `persist_every.is_some() && corpus_dir.is_some()` and the count is a multiple, `index.persist(&corpus_dir)` fires. Default `persist_every` stays `None` (callers opt in via [`with_corpus_dir`]) — flipping it globally would silently change behaviour for every test that uses `IngestionPipeline` without a corpus_dir. (2) Added `corpus_dir: Option<PathBuf>` field + `IngestionPipeline::with_corpus_dir(path)` builder. (3) **Dead-code purge along the bridge path:** the per-file `bridge_endpoints` accumulator was always discarded (`finalize_ingestion` rebuilds bridges from `all_files`). Removed the field from `CodeSymbolsResult` and `FileResult::Indexed`, removed the `bridge_linker` parameter from `extract_code_symbols`, `parse_and_store_file`, and `run_producer_consumer`, and dropped the per-file `linker.extract_all(...)` call inside `extract_code_symbols`. Net: per-file bridge extraction (a real CPU cost on code-heavy corpora) no longer runs; the authoritative full rebuild in `finalize_ingestion` is unchanged.
  - `cargo build --workspace`, `cargo clippy -p ministr-core --all-targets -- -D warnings -W clippy::pedantic`, `cargo test -p ministr-core` (1495 lib + ~270 integration, all pass) clean.
  - **Honest negatives:** the persist hook has no targeted unit test — verification relies on the existing 1495-test suite (which exercises every ingestion path with `persist_every=None`) plus chunk 5's operator smoke. Cloud-worker wiring (passing `corpus_dir` + `persist_every=Some(4)` from `cmd_indexer_worker` / `run_corpus_ingestion`) is **not** in this commit — moved to the backlog as a follow-up so chunk 4 stays atomic to "core plumbing + dead-code purge".
  - The actual cloud OOM lever (ONNX activations + per-file AST + claim graph held across `buffer_unordered` futures) was NOT addressed. Those are downstream of the embedder and parser, not the pipeline; surfaced as backlog items.

- [x] **Chunk 5 — Right-size worker post-streaming** — added `ministr-azure:jobMemory: 4Gi` to `Pulumi.prod.yaml` (down from the 8 GiB default in `index.ts`). `jobCpu` stays at the default 4: the doc conditioned the drop on a benchmark we haven't run. Added `just phase4-smoke` recipe — same functional sequence as `phase3-smoke` (demo-remote → restart serve pod → re-run demo-remote), with the comment block flagging OOM rollback semantics if a replica exits 137. Cost win at the right-sized spec: ~$2/h-when-running on 4 vCPU / 4 GiB vs ~$4/h on 4 vCPU / 8 GiB; idle is $0 either way via the KEDA 0-replica scaler. `npx tsc --noEmit` clean. **Operator-side verification still required** — `pulumi up` + `just azure-demo` against the live cluster — can't run here without Azure creds. *(Post-chunk: `phase4-smoke` collapsed into `azure-smoke`, the canonical ever-evolving smoke that `just azure-demo` calls at its tail. One command to test any phase.)*

- [x] **Chunk 6 — Observability + ops polish** — three deliverables shipped:
  - `just azure-orphans` recipe — auto-firewalls the flex-server (same pattern as `azure-psql`), runs a `cloud_corpora` left-anti-join against `indexer_jobs` filtered to `status='completed'` and prints rows with no terminal-success job, then `az role assignment list --scope <storage>` filtered to `principalName==''` to surface orphan principals. The auto-firewall + `trap` cleanup means Ctrl-C still drops the rule.
  - **SSE terminal-event wire-shape fix + summary stat.** PHASE3 chunk 4's `queue_progress_stream` emitted `status="error"` for `IndexJobStatus::Failed`, but `cloud_demo::stream_progress` checks `evt.status == "failed"` — so a failed job in cloud mode used to fall through to the "progress stream closed by server" branch and exit `Ok` (silent failure). Fixed in `ministr-daemon/src/daemon.rs:queue_progress_stream` to emit `"failed"` and to pass through `IndexJobSnapshot::error` on the terminal frame. Added an `error: Option<String>` field (serde-default, skip_serializing_if_none) to `ministr-api::corpus::IngestionProgressEvent`. `cloud_demo` now surfaces the error cause + final file count in the terminal line; the non-cloud `progress_stream` writes `error: None` (status code 2 = complete only).
  - `just azure-cost` recipe — calls `az containerapp job execution list --output json`, pipes through `scripts/azure-cost-summary.py` (Python kept out-of-just because just's whitespace-sensitive recipe-body parsing conflicts with Python's indentation). Aggregates execution-seconds per day, applies a configurable `$/active-second` rate (default $0.0001156 = 4 vCPU + 4 GiB at May-2026 East-US Consumption pricing; override via `COST_PER_SECOND=…`), and prints a projected $/month. A quiet KEDA-scaled stack should report near-zero — directly validates chunk 1's "≈$0/mo when idle" claim.
  - `cargo build --workspace`, `cargo test --workspace --lib` (237 + per-crate passes), `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`, `npx tsc --noEmit`, `just --list` (new recipes parse and show up) all clean.
  - **Honest negative:** the SSE summary-stat behaviour can't be verified end-to-end without driving a real failed cloud ingest. The wire-shape fix is a pure rename + an `error` plumb-through; the demo client's failed-branch was already there and now actually fires. Operator can confirm by submitting an intentionally bad clone-url and watching `just demo-remote` exit with the cause instead of "stream closed by server".

## Open questions

- ~~**KEDA `postgresql` scaler auth in ACA.**~~ *Resolved in chunk 1.* Use the standard ACA `JobScaleRule.auth` array mapping the `pg-url` secret onto `triggerParameter: "connection"` — the same TriggerAuthentication pattern KEDA uses upstream. No `connectionFromEnv` needed.
- ~~**`HnswIndex` incremental add semantics.**~~ *Resolved in chunk 3.* `HnswIndex::insert` is already incremental (the consumer calls it per batch today). `HnswIndex::persist` is atomic via stage-into-tmp + fsync + rename (per the existing `atomic HNSW persist with crash-recovery backup` change), safe to call mid-build. The `with_config` constructor is one path, not the hot path.
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
- [x] **Cloud worker streams via `persist_every`** — `run_corpus_ingestion` gained a `streaming_persist_every: Option<usize>` parameter. `cmd_indexer_worker` opts in with `Some(4)`; `cmd_index` + `infra::spawn_*` + `spawn_config_watcher` pass `None` to preserve PHASE3 bundle-at-end. When opted in, the pipeline gets `with_corpus_dir(ctx.index_dir)` + `BatchIngestionConfig { persist_every: Some(4), batch_size: 4 }`, so chunk 4's persist hook fires every 4 indexed files. cargo build --workspace, clippy --pedantic, ministr-cli + ministr-core tests all clean. Now actually exercises the chunk-4 plumbing on cloud; chunk 5's right-size is no longer functionally blocked.
- [ ] **ONNX activation peak** — likely the real OOM driver on anyhow (`~600 MB per file` in PHASE4 doc analysis). Options: smaller batch into the embedder; spawn_blocking the per-batch embed call so the runtime can free Tokio-task locals between batches; or constrain the producer's `buffer_unordered` concurrency from `default_concurrency()` to a lower fixed value under cloud-worker memory pressure.
- [ ] **Producer-task lifetime** — each in-flight `parse_and_store_file` future holds an AST + claim graph for the file's duration. `buffer_unordered(concurrency)` keeps `concurrency` futures alive at once. On a 4 GiB worker this caps how many large files can be in flight; the current `default_concurrency()` (likely #cpus) may be too aggressive. Profile rss vs concurrency to find a safe upper bound.

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
