# Cloud Phase 3 — serve / worker split

> **Status: shipped (May 2026). Active follow-on work lives in [PHASE4.md](./PHASE4.md).**
>
> All six chunks here landed end-to-end on the deployed cluster. Post-deploy
> smoke surfaced two structural issues that aren't fixes to this phase but
> a new architectural delta — see PHASE4 for event-driven scaling
> (replaces the chunk-6 cron-poll) and streaming ingestion (fixes the
> OOM that chunk 3's worker hit on a real-sized corpus).
>
> This doc is now historical record; do not modify it in place.


PHASE2 (May 2026) made corpus *bundles* durable via blob. Implementation
surfaced two architectural smells the chunks-as-shipped do not address:

1. **`corpora.json` is still pod-ephemeral.** Registry-of-registrations
   (the *list* of which corpora exist) lives at
   `<data_dir>/corpora.json` and disappears on pod recycle. PHASE2's
   `restore()` is a no-op on cloud cold start; recovery only happens
   when a client re-registers the same source paths.
2. **One pod does two incompatible jobs.** The serve replica handles
   HTTP / auth / queries (latency-sensitive, low-CPU) *and* runs ONNX
   embedding (throughput-bound, CPU-saturating). The first PHASE2
   smoke test demonstrated this: embedding `dtolnay/anyhow` on a
   `0.5 CPU` replica caused concurrent reads to 503 at the ACA
   ingress, even though no pod crash occurred.

This doc is the design pass for separating the two roles and moving
the registry into Postgres.

## Existing scaffolding that PHASE3 re-uses

PHASE3 is mostly wiring already-built pieces:

| Piece | Where | What we re-use |
|---|---|---|
| `JobQueue` trait, `Job`, `JobStatus`, `JobProgress`, `JobTrigger` | `ministr-mcp/src/admin/jobs/mod.rs` | Generic queue shape, already used by Atlas |
| `SqliteJobQueue` / `InMemoryJobQueue` | `ministr-mcp/src/admin/jobs/` | Backends; add `PostgresJobQueue` alongside |
| `cmd_atlas_reindex` worker pattern | `ministr-cli/src/commands.rs` | Entrypoint shape — single-shot run, poll queue, exit |
| `ministrv2-indexer` ACA Job (4 vCPU / 8 GiB) | `deploy/azure/lib/job.ts` | The worker container itself, with `ENTRYPOINT_MODE=index` |
| `BlobBackendSink` + `CorpusBlobStore::upload_corpus` | `ministr-cloud/src/blob_sink.rs`, `blob.rs` | Same bundle-export-and-upload path, just called from the worker instead of the serve pod |
| `github_webhook` enqueue pattern | `ministr-mcp/src/admin/webhook.rs` | One call site that already converts an HTTP event into a job |

## Goal

Serve pod stays small (0.5–1 CPU) and **never** runs the embedder.
Ingest (clone + parse + embed + bundle + upload) runs in the ACA Job
container, which scales from zero on demand. Postgres is the
authoritative source for "which corpora exist" so every pod sees the
same registry across restarts.

## Architecture

```
  ┌─────────────────┐                ┌────────────────────────┐
  │  Serve pod      │   enqueue      │  Postgres              │
  │  (0.5-1 CPU)    │ ─────────────▶ │   corpora              │
  │                 │                │   index_jobs (queue)   │
  │  - HTTP/MCP     │   read         │   oauth_*              │
  │  - OAuth        │ ◀───────────── │                        │
  │  - Queries      │                └────────────────────────┘
  │  - Status       │                          ▲
  └─────────────────┘                          │ claim/update
          │                                    │
          │ blob.download                      │
          ▼                                    │
  ┌─────────────────┐                ┌────────────────────────┐
  │  Azure Blob     │ ◀── upload ─── │  Indexer ACA Job        │
  │   ministr-      │                │  (4 vCPU / 8 GiB,       │
  │   corpora       │                │   scale-to-zero,        │
  │                 │ ── download ─▶ │   ENTRYPOINT_MODE=index)│
  └─────────────────┘                │                         │
                                     │  - poll Postgres queue  │
                                     │  - clone + index        │
                                     │  - upload bundle        │
                                     │  - mark job done        │
                                     └────────────────────────┘
```

Lifecycle of a new registration:

1. Client `POST /api/v1/corpora` against the serve pod.
2. Serve handler `INSERT INTO corpora (id, tenant_id, paths, status='pending')` and `INSERT INTO index_jobs (corpus_id, trigger, priority)`. Returns `(corpus_id, status: "pending")` immediately.
3. Worker (running as ACA Job, started by cron or by serve calling the Azure REST `/jobs/{name}/start` API) wakes, claims the job (`UPDATE … SET status='running' WHERE id=? AND status='pending'`), runs the existing `indexer::run` against a corpus dir on the job's pod-local disk, calls `BlobBackend::upload_corpus`, and `UPDATE index_jobs SET status='done'`.
4. Serve pod's `GET /api/v1/corpora/{id}/progress` SSE reads `JobProgress` from Postgres and streams it to the client.
5. Serve pod's query handlers (`ministr_survey`, `ministr_read`, …) hit `BlobBackend::download_corpus` on first use to populate `<data_dir>/corpora/<id>/` from blob, then keep it in-memory.

## Chunks (atomic, one per `/roadmap` invocation)

- [x] **Chunk 1 — Postgres-backed corpus registry**
  - New migration `ministr-cloud/migrations/0003_corpus_registry.sql` creates
    `cloud_corpora (corpus_id, tenant_id, paths JSONB, display_name, status, created_at, updated_at)` with a tenant+created_at index.
  - **Naming deviation**: distinct from F1.2's UUID-keyed `corpora`
    (which has a CHECK requiring exactly one of owner_user_id/owner_org_id
    and a string≠UUID identity mismatch). The two will be joined or
    merged when multi-tenant ACL lands; until then `cloud_corpora`
    is the working pod-shared registry.
  - **`corpus_roots` mirror deferred** — not needed by chunk 1's
    register/restore surface; bundle manifests are still built by
    `ministr-cloud::blob_sink::build_manifest_from_corpus_dir` reading
    the on-disk SQLite. Promote to chunk 6 backlog if a future
    direct-read use case appears.
  - New trait `ministr-api::corpora_repo::CorporaRepo` — `dyn`-safe with
    `BoxFuture`-returning upsert/remove/list. New impl
    `ministr-cloud::PostgresCorporaRepo` against the deadpool pool.
  - `CorpusRegistry` gained `corpora_repo: OnceLock<Arc<dyn CorporaRepo>>`,
    `set_corpora_repo()`, and `notify_repo_*` helpers wired into
    register / unregister / update_corpus_paths and into restore()
    (repo source wins over `corpora.json` when set; dead-entry pruning
    also calls `repo.remove` so a stale `/tmp/...` row doesn't keep
    reappearing on every pod boot).
  - `cmd_serve_http` hoists cloud_pool above the registry build and
    wires `PostgresCorporaRepo` before `restore()`. Self-hosted serve
    leaves the OnceLock empty — `corpora.json` stays the source of
    truth.
  - Verified: `cargo build --workspace` clean; 10 ministr-api lib
    tests (2 new), 113 ministr-cloud lib tests (1 new dyn-compat),
    61 ministr-daemon lib tests — all pass; pedantic clippy clean
    across ministr-api / ministr-cloud / ministr-daemon / ministr-cli.

- [x] **Chunk 2 — `PostgresJobQueue` + tenant `JobTrigger::Tenant`**
  - **Scope-reduction discovered during exploration**: `PostgresJobQueue`
    already exists at `ministr-mcp/src/admin/jobs/postgres.rs` (shipped
    by a prior PR; gated by `#[allow(dead_code)]`). It implements full
    `JobQueue` + `FOR UPDATE SKIP LOCKED` + priority lane + the
    `indexer_jobs` table via on-demand `ensure_schema`. Tests live
    behind `#[ignore = "needs MINISTR_TEST_PG_URL"]` — **PHASE3's
    `pg-embed` reference was inaccurate**; the queue is exercised
    against a real Postgres in CI, not an embedded one.
  - Net new this chunk: `JobTrigger::Tenant { paths: Vec<String>,
    clone_url: Option<String> }` variant in `JobTrigger`, serialised
    with the same `tag = "kind"` snake_case envelope as `Manual` /
    `Github`. `#[allow(dead_code)]` until chunk 4 wires the
    serve-pod enqueue path.
  - Storage rides along automatically — the queue's `data TEXT`
    column holds the JSON-serialised `Job` (with its embedded
    `JobTrigger`), so no migration needed.
  - Verified: 9 `ministr-mcp::admin::jobs` lib tests pass (2 new
    round-trip tests for `JobTrigger::Tenant`, with and without
    `clone_url`); pedantic clippy clean on `ministr-mcp`; workspace
    builds clean.

- [x] **Chunk 3 — `cmd_indexer_worker` entrypoint**
  - New `ministr-cli/src/commands.rs::cmd_indexer_worker` —
    single-shot, queue-driven. Opens the cloud Postgres queue
    (`MINISTR_PG_URL` required), claims one job via
    `PostgresJobQueue::claim_next`, dispatches on
    `JobTrigger::Tenant` (other triggers return `Failed` with an
    explanatory message), resolves sources from `clone_url`
    (preferred) or `paths`, runs `infra::init_infrastructure` +
    `ingestion::run_corpus_ingestion` (the existing pipeline already
    handles git URLs via `classify_corpus_path`, so the worker does
    not need its own `git clone` shell-out), then uploads the
    bundle under the deterministic `job.corpus_id` (not
    `ctx.corpus_dir`'s hashed name) and marks the job `Completed`.
    `claim_next` returning `None` exits clean — ACA Job cron-poll
    no-work tick.
  - **ENTRYPOINT_MODE deviation**: chunk text proposed re-purposing
    `ENTRYPOINT_MODE=index`, but `ministr index` already exists as
    the toml-driven local indexer (used by self-hosted serve and
    local CI). Instead added a new `indexer-worker` subcommand and
    a new `ENTRYPOINT_MODE=indexer-worker` case in
    `deploy/docker-entrypoint.sh`. Chunk 6 will switch the ACA
    Job's Pulumi env var to the new mode.
  - **Visibility uplift**: bumped the `ministr-mcp::admin::jobs`
    module surface from `pub(crate)` to `pub` (trait, types, all
    three backends, constructors). Exposed
    `ministr_cloud::build_manifest_from_corpus_dir` +
    `ManifestBuildError`. This is what makes the worker callable
    from `ministr-cli`.
  - **Deferred** (chunk 6 cleanup): physically moving
    `BlobBackendSink` + the serve-side completion reactor into the
    worker. The worker calls `BlobBackend::upload_corpus` directly
    inline; the serve pod's PHASE2 sink reactor stays in place
    until chunk 4/6 makes it vestigial.
  - Verified: workspace builds clean including ministr-app;
    pedantic clippy clean across the five touched crates
    (ministr-api / ministr-cloud / ministr-daemon / ministr-mcp /
    ministr-cli); ~1946 workspace lib tests pass (no new tests in
    this chunk — the worker is invocation-pattern code most
    naturally exercised by the chunk 6 cloud smoke).

- [x] **Chunk 4 — Serve pod enqueues instead of running `indexer::run`**
  - New `ministr-api::IndexJobSink` trait (`BoxFuture`-returning
    `create_pending` + `latest_for_corpus`). Lives in ministr-api
    because ministr-daemon cannot depend on ministr-mcp (reverse arrow
    established).
  - New `ministr-cloud::PostgresIndexJobSink` — in one transaction
    UPSERTs `cloud_corpora` + INSERTs an `indexer_jobs` row with the
    chunk-2 `JobTrigger::Tenant` shape. JSON envelope identical to
    `PostgresJobQueue::enqueue`; the worker's `claim_next` finds it
    unchanged.
  - `AppState::index_job_sink` field + `with_index_job_sink` builder.
  - `register_corpus` and `clone_repo` handlers branch on sink-presence:
    in cloud mode they enqueue + return `(corpus_id, indexing_started=true)`
    immediately. Self-hosted serve keeps the inline-register path.
  - `ingestion_progress` SSE switches in cloud mode to a 500ms-interval
    `queue_progress_stream` that polls Postgres and maps
    `IndexJobStatus` → the existing `IngestionProgressEvent` wire shape.
    Terminal status closes the stream (same behaviour as before).
  - **Decision recorded inline (per chunk text)**: clone-route response
    keeps the same JSON shape — `clone_dir` / `commit_sha` / `branch`
    return as empty placeholders in queue mode (worker fills the actual
    values; demo client watches progress for them).
  - **Limitation (deferred)**: GitHub App installation-token cloning
    returns `501 Not Implemented` in queue mode — token would expire
    before the worker dequeues. PAT-in-URL clones still work. A future
    refinement adds `installation_id` to the Tenant trigger so the
    worker mints at clone time.
  - **Vestigial wiring (deferred to chunk 6 cleanup)**: PHASE2's
    serve-side `BlobBackendSink` + completion reactor become
    unreachable in cloud mode — the registry never spawns
    `indexer::run`, so `notify_complete` never fires. Harmless until
    physically removed.
  - Verified: workspace builds clean including ministr-app;
    pedantic clippy clean across `ministr-api` / `ministr-cloud` /
    `ministr-daemon` / `ministr-mcp` / `ministr-cli`; ~1949 lib tests
    pass (3 new: 1 `IndexJobSink` round-trip in ministr-api, 2 in
    ministr-cloud for dyn-compat + job-id prefix).

- [x] **Chunk 5 — On-demand bundle restore in the serve pod**
  - New `ministr-api::CorpusRestorer` trait (`BoxFuture`-returning
    `download` + `CorpusRestoreError::{NotFound, Backend}`). Same
    dep-direction reason as PHASE3 chunk 4: lives in ministr-api so
    ministr-daemon can consume without depending on ministr-cloud.
  - New `ministr-cloud::BlobCorpusRestorer` wrapping
    `Arc<BlobBackend>` — calls `BlobBackend::download_corpus` and maps
    failures through a string-shape probe to distinguish NotFound
    from other backend errors. (The private `is_not_found` helper in
    `blob.rs` has the typed check; promoting it to pub `BlobError`
    method is a small follow-up — see backlog.)
  - `CorpusRegistry` gained: `corpus_restorer: OnceLock`, a
    `restore_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>` for
    per-corpus serialisation, a `set_corpus_restorer` setter, and two
    new methods:
    - `ensure_present(corpus_id)` — fast-path map check → repo lookup
      → per-corpus mutex → `restorer.download()` → `register_restored`,
      with a double-check after lock acquisition to handle TOCTOU.
    - `register_restored(corpus_id, paths, display_name)` — inserts a
      `CorpusHandle` from on-disk data without spawning `indexer::run`
      or a watcher (the bundle is fully indexed; the source files
      don't live on the serve pod).
  - `CorpusRegistry::get` now calls `ensure_present` on a miss,
    transparently triggering lazy restore. Self-hosted serve (no
    restorer wired) preserves the historical `NotFound` behaviour.
  - `cmd_serve_http` **removed** PHASE2's boot-time bulk download
    (the `list_corpora` + per-id `download_corpus` loop); refactored
    `blob_backend` into `blob_backend_arc` so both the new restorer
    and the existing PHASE2 sink share one `Arc<BlobBackend>`.
  - **Backlog item discovered**: promote `BlobError::is_not_found` to
    a public helper so the restorer can use the typed check instead
    of string-shape matching.
  - Cache eviction policy: out of scope — corpora stay on disk until
    pod recycle. Revisit if `/data` pressure becomes real.
  - Verified: workspace builds clean including ministr-app;
    pedantic clippy clean across the five touched crates; ~1950 lib
    tests pass (1 new: `CorpusRestorer` dyn-compat in ministr-api).

- [x] **Chunk 6 — ACA Job trigger + smoke (Pulumi side)**
  - `deploy/azure/lib/job.ts` rewritten end-to-end:
    - `triggerType: "Schedule"` with
      `scheduleTriggerConfig: { cronExpression: "*/1 * * * *", replicaCompletionCount: 1, parallelism: 1 }`.
      Cron-poll option (A) per the recommendation; option (B) — serve
      pod triggers via Azure REST — stays in backlog.
    - `ENTRYPOINT_MODE` env switched from `index` → `indexer-worker`
      (the chunk-3 subcommand).
    - `MINISTR_CORPUS_PATHS` env dropped — the worker ignores it
      (sources come from the popped `JobTrigger::Tenant`).
    - `MINISTR_PG_URL` secret env wired from a new
      `pgConnectionString?: pulumi.Input<string>` input. Mirrors the
      app's pg-url secret pattern.
    - `identity: { type: "SystemAssigned" }` so
      `ManagedIdentityCredential` can authenticate blob ops; the job
      now returns `principalId` alongside `name`.
  - `deploy/azure/index.ts` threads `postgres?.pgConnectionString`
    into the job + calls `grantBlobDataContributor` on the new
    indexer principal — mirrors the queryApp grant.
  - **Verified**: `tsc --noEmit` clean. Cloud smoke (below) is
    operator-driven — I did not run `pulumi preview` / `pulumi up`
    (no Azure creds in this session); the schedule trigger and the
    new role assignment land only on the operator's next deploy.

### Smoke acceptance (operator-run after `pulumi up`)

```sh
just azure-demo         # idempotent: provision-if-needed + push + roll + azure-smoke
```

`just azure-demo`'s tail step `just azure-smoke` (the canonical, ever-evolving smoke
extended by each phase — was `phase3-smoke` at PHASE3, then `phase4-smoke`, now
just `azure-smoke`) runs three steps in sequence:

1. **`just demo-remote`** — clone-url → `POST /api/v1/corpora` returns
   `(corpus_id, status: pending)` instantly (chunk 4: serve pod no
   longer ingests inline). Progress SSE streams from Postgres
   `indexer_jobs` until the scheduled worker (chunk 6) drains the
   queue ≤60s later and `status=complete`. A survey query against
   the corpus succeeds.
2. **`just azure-restart-app`** — drops pod-local `/data`.
3. **`just demo-remote`** again — same `CLONE_URL` ⇒ deterministic
   `corpus_id` hits the existing `cloud_corpora` row (chunk 1); the
   survey query lazy-downloads the bundle from blob (chunk 5) and
   succeeds. Proves end-to-end durability across pod recycle.

Useful side recipes during smoke:

- `just azure-logs` — tail the serve pod (Streamable HTTP + REST).
- `just azure-logs-indexer` — tail the scheduled worker's recent
  ticks (claim_next + ingest + upload + finish).
- `just azure-status` — stack outputs + `/healthz` probe.

## What gets superseded from PHASE2

- **PHASE2 chunk 4 boot-time bulk download** — removed in PHASE3 chunk 5 in favor of on-demand restore. Wasteful at boot, doesn't scale once a single pod hosts many tenants' corpora.
- **PHASE2 chunk 3 completion channel + chunk 1 `AppState::blob_sink`** — *moved*, not removed. The same `BlobBackendSink` runs in the worker; the serve pod no longer needs either field. Remove the serve-side wiring in chunk 6 cleanup.
- **`set_completion_sink` on `CorpusRegistry`** — the registry no longer runs ingest, so the hook has no caller on the serve side. Keep `pub` for any future use by callers like Tauri-local mode; mark it `#[doc(hidden)]` cloud-side.

## What stays from PHASE2

- `BlobSink` trait (`ministr-api`).
- `BlobBackendSink` impl + `build_manifest_from_corpus_dir` (`ministr-cloud`).
- `BlobBackend::{upload_corpus, download_corpus, list_corpora}` and the underlying `CorpusBlobStore` — unchanged contract, just called from a different process.
- The bundle wire format (`<id>/manifest.json` pointer + `<id>/<version>.ministr-index`).

## Open questions

- **Per-tenant corpus dir prefix on blob.** Today bundles live at
  `corpora/<corpus_id>/…` (single-tenant-shaped). When multi-tenant
  goes live we'll want `tenants/<tenant_id>/corpora/<corpus_id>/…`.
  Probably folded into chunk 1's schema change.
- **Worker concurrency.** ACA Job `manualTriggerConfig.parallelism: 1`
  today — one job at a time. Bumping to 2–4 needs both a Pulumi tweak
  and the `PostgresJobQueue` claim to use `FOR UPDATE SKIP LOCKED`.
- **Cleanup of dead jobs.** If a worker crashes mid-job the row stays
  `running` forever. Need a `claimed_at` timestamp + a "reclaim jobs
  older than N minutes" sweeper. Out of scope for v1; backlog.

## Verify

- Rust: `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`
- Postgres migrations apply on a clean DB: `pulumi -C deploy/azure up --yes` against a fresh stack, check the boot log for `cloud postgres migrations applied`.
- Cloud smoke per chunk 6.

## Discovered / backlog

- [ ] **Promote `BlobError::is_not_found` to public** — the chunk 5
  `BlobCorpusRestorer` currently string-shape probes for `BlobNotFound`
  / `404` to distinguish NotFound from generic backend errors. The
  private `is_not_found` helper in `ministr-cloud/src/blob.rs` already
  does this typed; expose it as `BlobError::is_not_found(&self) -> bool`
  and switch the restorer to use it.
- [ ] **Per-tenant blob prefix** — see Open questions.
- [ ] **Worker concurrency > 1** — needs `FOR UPDATE SKIP LOCKED` + Pulumi parallelism bump.
- [ ] **Crashed-worker reclaim sweeper** — `claimed_at` + janitor.
- [ ] **Azure REST trigger from serve pod (chunk 6 option B)** — latency win, costs an extra Azure SDK dep + RBAC role.
- [ ] **Disk eviction in the serve pod** — restored corpora pile up under `/data` until pod recycle.
