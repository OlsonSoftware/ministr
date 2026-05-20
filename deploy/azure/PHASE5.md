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

- [x] **Chunk 2 — Empty-index persist gate** *(code + tests landed; live `just azure-demo` log clean-up pending operator run)*
  - [x] `ministr-core/src/ingestion/pipeline.rs::run_producer_consumer`
    — gate added: when `persist_every` boundary fires but
    `index.is_empty()`, skip the persist call and log at TRACE. Race
    description preserved in the inline comment so the why survives.
  - [x] Regression tests in `phase5_chunk2_persist_gate_tests`: pins
    that (a) a freshly-built `HnswIndex` reports `is_empty=true` (the
    state the parser-side counter races into), and (b) calling
    `persist()` on the empty index is the failure mode the gate
    avoids — confirmed by the "fail empty, succeed with one vector"
    behavioural assertion. Driving the full pipeline was rejected as
    too heavy per PHASE4 chunk 4's "no targeted unit test" pattern.
  - [x] Workspace verification clean: 1962 lib tests passing
    (1502 in `ministr-core`, +2 new); workspace clippy pedantic clean.
  - [ ] **Operator action remaining**: confirm `just azure-demo` log
    no longer shows the `failed to dump HNSW: unexpected error` WARN
    during streaming ingest.

- [x] **Chunk 3 — Streaming progress heartbeat** *(code + tests landed; live `just azure-demo` SSE confirmation pending operator run)*
  - **Honest revision.** PHASE5.md's premise that "the heartbeat got
    dropped" turned out to be wrong on code-read: the 500ms reporter
    in `cmd_indexer_worker` (commands.rs:2148-2186) is still wired and
    has been since PHASE3 chunk 3. The actual gap was a **wire-shape
    clip**: `JobProgress` only carried `(stage, total_files,
    processed_files, current_file)`, and `queue_progress_stream`
    hardcoded `sections_done = embeddings_total = embeddings_done =
    0`. So the streaming consumer's per-batch
    `progress.add_embeddings_done(count)` updated the in-memory
    `IngestionProgress` but never reached the SSE. Result: the SSE bar
    plateaued at parser-side N/N during the long embedder phase
    (which manifests as "stuck" to a user, and as 0/0 on an
    empty-corpus run).
  - [x] Extended `JobProgress` (`ministr-mcp::admin::jobs`) with three
    new fields: `sections_done`, `embeddings_total`, `embeddings_done`.
    All three carry `#[serde(default)]` so in-flight PHASE4-era rows in
    `indexer_jobs.data` (TEXT JSON column) deserialise without
    migration.
  - [x] Extended `IndexJobSnapshot` (`ministr-api::index_job_sink`)
    with matching fields. Same `serde(default)` backward-compat.
  - [x] `PostgresIndexJobSink::create_pending` seeds the new fields in
    the initial blob; `latest_for_corpus` lifts them out via the
    extracted `snapshot_from_blob` helper.
  - [x] `cmd_indexer_worker` reporter samples
    `progress.{sections_done, embeddings_total, embeddings_done}` and
    writes the full snapshot to `queue.update_progress`. The streaming
    consumer's existing per-batch `add_embeddings_done(count)` is the
    "heartbeat at the batch boundary" — no new callback was needed.
  - [x] `daemon::queue_progress_stream` populates the
    `IngestionProgressEvent` from the snapshot's new fields (was
    hardcoded `0` for all three pre-chunk-3).
  - [x] Round-trip regression tests in
    `ministr-cloud/src/index_job_sink.rs::tests`:
    `snapshot_round_trips_phase5_chunk3_fields` pins the JSON shape;
    `snapshot_back_compat_with_phase4_blobs` pins that PHASE4-era
    rows still parse with new fields defaulting to 0.
  - [x] Workspace verification clean: **1965** lib tests passing (+3
    new); workspace clippy pedantic clean.
  - [ ] **Operator action remaining**: confirm `just azure-demo` SSE
    shows live `embeddings_done` progress during the embedder phase
    (the field the UI should render as the primary bar; client-side
    UI refresh to follow if needed).

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
