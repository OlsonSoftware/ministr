# Cloud Phase 5 — actually-event-driven trigger + PHASE4 live-deploy fallout

PHASE4 (May 2026) added streaming ingestion and swapped the cron trigger for a
KEDA `postgresql` scaler. Streaming is correct and stays. The KEDA trigger is
*not* event-driven — it polls Postgres every 5s, just faster than the cron.
The live PHASE4 deploy also surfaced two pipeline bugs that PHASE4's
verification scaffolding missed.

PHASE5 corrects all three.

## What this phase fixes

| Item | Root cause | Fix in PHASE5 |
|---|---|---|
| KEDA poll is the primary trigger | PHASE3 chunk 6 deferred "option B" (ARM-start from serve pod) for one RBAC role. PHASE4 chunk 1 doubled down by replacing cron with KEDA polling instead of fixing the framing. | Chunk 1 — ARM `POST /jobs/{name}/start` from the serve pod on enqueue; KEDA degrades to a 5-min slow-poll safety net. |
| Streaming HNSW persist fires on an empty index | PHASE4 chunk 4 instrumented `persist_every` on `files_indexed` (parser-side counter), which races ahead of the embedder. By the time the parser hits 4 files the embedder hasn't yet pushed any vectors into HNSW, so persist fails with `nb point 0`. | Chunk 2 — gate `persist()` on `index.len() > 0` (PHASE3 Fix A's spirit, applied to the streaming hook). |
| Streaming worker reports `0/0` on the SSE forever | PHASE3 Fix B added a per-500ms `queue.update_progress` heartbeat. The PHASE4 chunk 4 refactor moved code paths around and the heartbeat got dropped from the streaming path. | Chunk 3 — re-add the heartbeat at the streaming consumer's batch boundary, plus a unit test guarding against future regression. |

## Why this is PHASE5 and not patches on PHASE4

PHASE4 is "done" in the sense that all six chunks shipped and the worker is
ingesting on the live cluster. But the trigger substrate is being replaced
(not patched), and the two streaming bugs are big enough to warrant a smoke
gate before they get called done. New phase keeps the per-phase honesty.

## Goal

One architectural correction + two stability fixes:

1. **Genuinely event-driven trigger.** Replace KEDA-as-primary with a
   direct ARM `POST /subscriptions/.../jobs/{name}/start` call from the
   serve pod after the `indexer_jobs` INSERT succeeds. KEDA stays in
   `lib/job.ts` as a 5-minute safety net so a transient ARM failure
   doesn't strand a row. Latency drops from "KEDA poll cycle" to "ARM
   round-trip" (~1-2s). Empty-tick load drops to zero (the safety-net
   poll is 12 queries/hour, not 720).
2. **Empty-index persist gate.** `IngestionPipeline::run_producer_consumer`
   only calls `index.persist()` when the index has actually accumulated
   vectors.
3. **Streaming progress heartbeat.** Re-establish per-500ms
   `queue.update_progress` calls during streaming ingestion so the SSE
   shows real progress instead of `0/0`.

## Architecture

```
                ┌────────────────────────────────┐
                │  Postgres (existing)           │
                │   indexer_jobs                 │
   POST /api/v1/corpora                          │
        │       │                                │
        ▼       │                                │
  ┌─────────────┤◄─── INSERT ────────────────────┤
  │  Serve pod  │                                │
  │  (0.5 vCPU) │                                │
  │             │                                │
  │             │     ▲   FOR UPDATE SKIP LOCKED │
  │             │     │   ORDER BY priority,     │
  │             │     │           created_at     │
  │             │     │                          │
  │  ARM-start  │     │                          │
  │  /jobs/start├─────┼──────────────────────────┤   ◄── fast path
  │  (PHASE5    │     │                          │
  │   chunk 1)  │     │   KEDA postgresql        │
  │             │     │   slow-poll safety net   │   ◄── safety net
  │             │     │   (every 5 min)          │       (~12/hr)
  └──────┬──────┘     │                          │
         │            └──────┬───────────────────┘
         │                   │
         ▼                   ▼
  ┌──────────────┐   ┌─────────────────────────┐
  │  Azure Blob  │   │  Indexer Job (Event)    │
  │ corpora      │◄──┤  2 vCPU / 4 GiB         │
  └──────────────┘   │  scale 0→N on queue     │
                     └─────────────────────────┘
```

Concretely changes from PHASE4:

- New crate dep on the Azure REST surface (e.g. `azure_mgmt_appcontainers` or a
  hand-rolled `reqwest` call against the ARM endpoint — pick whichever is
  lighter on the serve pod's binary size and feature flags).
- New Pulumi role assignment: serve pod's SystemAssigned MI gets `Container
  Apps Jobs Operator` (or the minimal action set: `Microsoft.App/jobs/start/action`)
  scoped to the indexer Job. **This is the one RBAC role we should have added
  in PHASE3 chunk 6.**
- `lib/job.ts`: `pollingInterval: 5` → `300`; KEDA stays wired but slow.
- New `ministr-cloud::JobStartTrigger` trait + `AcaJobStartTrigger` impl that
  the serve pod calls fire-and-forget after enqueue. Mirrors the existing
  `IndexJobSink` pattern.
- `cmd_serve_http`: wire the trigger alongside the existing
  `PostgresIndexJobSink`.

## Chunks (atomic, one per `/roadmap` invocation)

- [x] **Chunk 1 — ARM-start trigger from serve pod** *(code + Pulumi landed; live `just azure-demo` smoke pending operator run)*
  - [x] Pulumi: `lib/job-start-role.ts` (NEW) grants serve pod MI the
    built-in `Container Apps Jobs Operator` role
    (`b9a307c4-5aa3-4b52-ba60-2b17c136cd7b`) scoped to the indexer Job,
    using the same deterministic UUID-v5 GUID pattern as
    `lib/role-assignment.ts`. Wired in `index.ts` via `grantJobsOperator`.
  - [x] Pulumi: `lib/job.ts` `pollingInterval: 5 → 300`. KEDA stays
    wired as the 5-min safety net.
  - [x] Pulumi: `lib/app.ts` injects `MINISTR_ACA_SUBSCRIPTION_ID`
    (`authorization.getClientConfigOutput().subscriptionId`),
    `MINISTR_ACA_RESOURCE_GROUP`, `MINISTR_ACA_INDEXER_JOB_NAME` env
    vars into the serve pod container.
  - [x] Rust: `ministr_api::JobStartTrigger` trait (NEW) +
    `JobStartError {Http, Imds, Arm{status,body}, Config}`. BoxFuture
    surface mirrors `IndexJobSink`; trait stays in MIT
    (`ministr-api`).
  - [x] Rust: `ministr_cloud::AcaJobStartTrigger` impl posts
    `https://management.azure.com/subscriptions/{sub}/resourceGroups/{rg}/providers/Microsoft.App/jobs/{job}/start?api-version=2026-01-01`
    with bearer token sourced from IMDS
    (`http://169.254.169.254/metadata/identity/oauth2/token?api-version=2018-02-01&resource=https://management.azure.com/`,
    `Metadata: true` header). Single-key token cache, `expires_on - 10min`
    proactive evict, `no_proxy()` so loopback IMDS never goes through
    `HTTPS_PROXY`. Hand-rolled reqwest — `azure_mgmt_appcontainers`
    SDK rejected for binary-size reasons.
  - [x] Rust: `PostgresIndexJobSink::with_start_trigger` builder
    attaches an `Arc<dyn JobStartTrigger>`; `create_pending` fires
    the trigger via `tokio::spawn` AFTER the txn commits, so a
    trigger failure never rolls the row back. `cmd_serve_http`
    builds the trigger when all three `MINISTR_ACA_*` env vars
    resolve, falls back to KEDA-only with a single warn at boot
    when any are absent.
  - [x] Verified: `cargo test --workspace --lib` (1960 passed, 0
    failed, 31 ignored Postgres-gated); `cargo clippy --workspace
    --all-targets -- -D warnings -W clippy::pedantic` clean;
    `cd deploy/azure && npx tsc --noEmit` clean. New tests:
    `job_start::tests` (happy-path round-trip against axum mock
    IMDS+ARM; ARM 403 surfaces as `JobStartError::Arm{403, body}`;
    dyn-trait dispatch).
  - [ ] **Operator action remaining**: `just azure-demo` end-to-end
    smoke. Expected: SSE picks up `status=running` within ~5s of the
    demo's POST (vs. up-to-5s under the old 5s KEDA cadence — the
    speed-up shows under load when multiple enqueues coincide; the
    decisive observable is "no empty-tick KEDA queries in the
    Postgres slow log").

- [ ] **Chunk 2 — Empty-index persist gate**
  - `ministr-core/src/ingestion/pipeline.rs::run_producer_consumer`: before
    calling `index.persist(&corpus_dir)`, check `index.len() > 0`. Skip
    persist if empty (no vectors to dump) and log at TRACE.
  - Unit test: drive the pipeline through a path that triggers
    `files_indexed % persist_every == 0` *before* any embedding batch
    lands; assert no persist call fires.
  - Verify: live `just azure-demo` log no longer shows the
    `failed to dump HNSW: unexpected error` WARN.

- [ ] **Chunk 3 — Streaming progress heartbeat**
  - Identify where PHASE3 Fix B's `queue.update_progress` heartbeat lived
    pre-PHASE4-chunk-4 (`ministr-cli/src/commands.rs::cmd_indexer_worker`
    or `run_corpus_ingestion`) and why the chunk-4 refactor dropped it from
    the streaming path. The honest answer might be "it was tied to a
    finalize step that no longer fires per-batch."
  - Re-establish at the streaming consumer's batch boundary: after each
    `index.insert()` batch, the worker bumps `queue.update_progress` with
    fresh `(processed_files, total_files, current_file)`.
  - Unit test in `ministr-mcp` or `ministr-cloud`: drive an ingestion that
    indexes 8 files with `persist_every=4`, assert the test sink saw
    ≥4 progress events (one per file or per batch — set the bar by what
    the heartbeat actually emits).
  - Verify: live `just azure-demo` SSE shows real per-file progress
    instead of stuck `0/0`.

## Open questions

- **ARM SDK vs hand-rolled reqwest.** `azure_mgmt_appcontainers` pulls a lot
  of generated code. A 50-line reqwest call (URL template + MI token +
  `POST /start`) is plausibly cleaner and smaller. Decide in chunk 1.
- **Worker MI vs serve pod MI for the role.** The role goes on the *serve
  pod*'s MI — it's the caller. Worker MI keeps its existing blob role only.
- **What if ARM start fails and KEDA also lags?** The 5-min safety net is
  the floor. A pathological case is "serve pod down + ARM down + KEDA
  scaler down" simultaneously — at that point the platform is broken in
  ways no trigger can fix.

## Discovered / backlog (inherited from PHASE4)

(Carried forward without re-evaluation; tackle once PHASE5 chunks land.)

- [ ] **Per-tenant blob prefix** — for multi-tenant pivot.
- [ ] **Worker concurrency > 1** — needs `parallelism: 2+` in
  `eventTriggerConfig` once a single replica's serial drain becomes a
  bottleneck.
- [ ] **Disk eviction in the serve pod** — restored corpora pile up under
  `/data` until pod recycle.
- [ ] **Promote `BlobError::is_not_found` to public**.
- [ ] **github-app `installation_id` in `JobTrigger::Tenant`**.
- [ ] **Managed embedding API** — `text-embedding-3-small` via Azure
  OpenAI. Unscheduled side track; only worth shipping if streaming hits
  a wall.
- [ ] **ONNX activation peak** — likely the real OOM driver. Options:
  smaller embedder batch; spawn_blocking the per-batch embed; constrain
  `buffer_unordered` concurrency.
- [ ] **Producer-task lifetime** — `buffer_unordered(default_concurrency())`
  may be too aggressive on 4 GiB. Profile rss vs concurrency.

## Verify

- Rust: `cargo build --workspace && cargo test --workspace --lib && cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`
- Pulumi: `cd deploy/azure && npx tsc --noEmit`
- Cloud smoke: `just azure-demo` (canonical end-to-end).

## Why this exists at all

The deferral pattern PHASE5 retires — "we'd need one extra RBAC role,
therefore the lazier architecture wins" — is recorded in the
[`feedback-no-rbac-deferral`](../../.claude/projects/-Users-alrik-Code-ministr/memory/feedback-no-rbac-deferral.md)
memory so future iterations don't repeat it. PHASE3 chunk 6 and PHASE4
chunk 1 are annotated with retroactive postmortems pointing here.
