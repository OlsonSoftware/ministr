# Cloud Phase 2 — durable corpus state via blob

Phase 1 (May 2026) provisioned the Postgres + Blob + RBAC infra in
Pulumi and dropped the SMB SQLite WAL hack. OAuth state survives pod
restarts via Postgres. **Corpus indexes still don't.** This doc is
the design pass for closing that gap.

## Chunks (atomic, one per `/roadmap` invocation)

- [x] **Chunk 1 — BlobSink trait + AppState wiring**
  - `ministr-api/src/blob_sink.rs` (new, MIT) — dyn-safe sync trait
    `BlobSink::enqueue_upload(corpus_id, corpus_dir)` mirroring the
    `UsageSink` pattern.
  - `ministr-daemon/src/state.rs` — `blob_sink: Option<Arc<dyn BlobSink>>`
    field on `AppState` + chainable `with_blob_sink()` constructor.
  - Verified: `cargo build -p ministr-api -p ministr-daemon` clean,
    8/8 ministr-api tests pass (2 new), clippy pedantic clean.
- [x] **Chunk 2 — BlobBackendSink impl in ministr-cloud**
  - `ministr-cloud/src/blob_sink.rs` (new, proprietary). Wraps
    `Arc<BlobBackend>` + an embedding `model_name`. `enqueue_upload`
    spawns a tokio task that opens `<corpus_dir>/content.db` to
    enumerate roots + count documents, loads
    `<corpus_dir>/index/` for `vector_count` + `dimension`, sets
    `bundle_version = Some(compute_bundle_version(&roots))`, then
    calls `BlobBackend::upload_corpus`. Errors warn-log + drop;
    no panics.
  - **Draft correction**: the §"Code changes / 2." snippet in this
    doc constructed `BundleManifest { bundle_version: None,
    corpus_roots: vec![], ... }`, which `CorpusBlobStore::upload_corpus`
    rejects with `BlobError::MissingBundleVersion`. The shipped sink
    mirrors `ministr-mcp::bundle_routes::build_manifest` (the
    canonical builder); `dimension` was dropped from the constructor
    because it's read from the actual HNSW index on disk, so a stale
    config value cannot desync the manifest from the index.
  - Verified: `cargo build -p ministr-cloud` clean,
    `cargo test -p ministr-cloud blob_sink` 2/2 pass (no-op-on-missing
    + dyn-compat), clippy pedantic clean.
- [x] **Chunk 3 — CorpusRegistry completion channel**
  - `ministr-daemon/src/registry.rs` — `completion_tx:
    OnceLock<UnboundedSender<(String, PathBuf)>>` field on
    `CorpusRegistry` + `set_completion_sink()` + `pub(crate)
    notify_complete()`. Mirrors the existing `coherence_sink`
    `OnceLock` pattern.
  - `ministr-daemon/src/indexer.rs` — `notify_complete(corpus_id,
    data_dir)` fires from the `Ok(stats)` arm of `indexer::run` after
    the resolver auto-heal block. One hook covers all three ingest
    paths: initial `register`, `update_corpus_paths` re-ingest, and
    `spawn_watcher`'s debounced re-run loop (all converge on
    `indexer::run`).
  - **Draft deviation A**: dropped the
    `with_completion_channel(mut self) -> (Self, Receiver)` builder
    from the design — `build_corpus_registry` already returns
    `Arc<CorpusRegistry>`, so a `mut self` builder is unreachable.
    The `OnceLock` + `&self` setter mirrors the local convention
    (`coherence_sink`) instead.
  - **Draft deviation B**: channel payload is `(corpus_id,
    corpus_dir)`, not bare `corpus_id`. Sending the path with the id
    avoids a registry lookup in the reactor that could race a
    concurrent `unregister`.
  - Verified: `cargo build -p ministr-daemon` clean, `cargo test
    -p ministr-daemon registry::` 12/12 pass (3 new), clippy
    pedantic clean (needed `#[allow(clippy::too_many_lines)]` on
    `indexer::run` — pre-existing sequential pipeline plus the
    durability hook crosses the 100-line heuristic; extracting a
    helper just to satisfy the lint would obscure the flow).
    Downstream crates `ministr-cli`/`ministr-mcp`/`ministr-cloud`
    all still build.
- [x] **Chunk 4 — cmd_serve_http boot restore + completion reactor**
  - `ministr-cli/src/commands.rs` (`cmd_serve_http`):
    - Boot-time: `build_blob_backend_from_env()` near the top; if
      `Some`, `list_corpora()` + per-id `mkdir` + `download_corpus`
      into `<data_dir>/corpora/<id>/` BEFORE the registry is built.
      Warn-log + continue on individual failures.
    - After registry `restore()` and `daemon_state` construction:
      build `BlobBackendSink` with `Arc<BlobBackend>` + `resolved_model`,
      install a completion channel via
      `corpus_registry.set_completion_sink(tx)`, attach to
      `daemon_state` via `with_blob_sink`, spawn a serial-drain
      reactor that calls `sink.enqueue_upload(corpus_id, corpus_dir)`.
  - **Design choice**: sink wired AFTER `restore()` so any cloud
    cold-start re-registration does NOT trigger a redundant upload —
    we just downloaded the bundle, blob IS the source of truth.
  - **Discovered gap (see backlog)**: `corpora.json` (the registry
    manifest of which paths are registered) is still pod-ephemeral.
    Bundles are durable; the LIST of which corpora exist is not.
    Cloud recovery flow: client re-registers with the same source
    paths → deterministic `corpus_id` → `create_handle` reuses the
    restored on-disk content.db + index/. No re-clone, no re-index.
  - Verified: `cargo build -p ministr-cli` clean, `cargo clippy
    -p ministr-cli --all-targets -- -D warnings -W clippy::pedantic`
    clean, full workspace `cargo build --workspace` clean. Cloud
    smoke (`just demo-remote` + `az containerapp revision restart` +
    re-run) deferred to the operator.

## Discovered / backlog

- [ ] **Persist `corpora.json` durably** — registry manifest is still
  pod-ephemeral. Without this, a pod restart loses the list of which
  corpora are registered even though their bundles are in blob.
  Workarounds today: client re-registers (fast path reuses restored
  on-disk data). Real fix: either store `corpora.json` as a blob
  sibling of the bundles, or move the manifest into Postgres
  alongside the OAuth state. Postgres is probably the right home —
  it already survives pod restarts and other registry-adjacent state
  could move there too.

## What's broken without Phase 2

A demo against the deployed cloud works on first run: clone → index →
watch progress. When ACA recycles the pod (image roll, node failure,
overnight idle scale-down), the pod-local `/data/.ministr/corpora/<id>`
tree is gone. Next request to the same corpus_id either:

1. **404s** if `corpus_registry.restore()` finds nothing on disk.
2. **Re-clones + re-indexes from scratch** if the client retries.

Either is a bad user experience for paid tenants. The blob container
`ministr-corpora` exists and the app has `Storage Blob Data Contributor`
RBAC, but `BlobBackend.upload_corpus` / `.download_corpus` have **zero
callers** in the live binary today.

## Goal

Make the corpus indexes durable across pod restarts. After each
successful ingestion, the `.ministr-index` bundle lives in blob. After
each pod boot, the registry's view of each tenant's corpora matches
what's in blob.

## Architecture

```
┌─────────────────────┐   download   ┌──────────────────────┐
│  Azure Blob Storage │ ◄──────────▶ │  Pod-local /data     │
│  ministr-corpora/   │   upload     │  .ministr/corpora/   │
│    <id>/manifest.json              │    <id>/             │
│    <id>/<ver>.bundle               │      (SQLite + HNSW) │
└─────────────────────┘              └──────────────────────┘
        ▲                                       ▲
        │                                       │
        │  ministr_cloud::CorpusBlobStore       │  ministr_core::bundle
        │  - upload_corpus(id, dir, manifest)   │  - export_bundle
        │  - download_corpus(id, target_dir)    │  - import_bundle
        │  - list_corpora()                     │
        └───────────────────────────────────────┘
                     │
                     │  via Option<Arc<dyn BlobSink>> on AppState
                     │
                     ▼
              ┌──────────────┐
              │ ministr-     │   triggers .enqueue_upload(id) when
              │ daemon       │   an ingestion task completes
              └──────────────┘
```

Open-core boundary: `BlobSink` trait lives in `ministr-api` (MIT),
impl wraps `BlobBackend` in `ministr-cloud` (proprietary), daemon
holds `Option<Arc<dyn BlobSink>>` — same pattern as `UsageSink` and
`InstallationTokenMinter`.

## Wire format

`CorpusBlobStore` already defines:
- `corpora/<id>/manifest.json` — atomic-swap pointer at current version
- `corpora/<id>/<version>.ministr-index` — versioned bundle (zstd-tar)

Bundle version computed from `compute_bundle_version(&corpus_roots)`.
Same on the upload and download sides — no new format work.

## Code changes

### 1. `ministr-api/src/blob_sink.rs` (new, MIT)

```rust
//! Durable corpus persistence hook.
//!
//! [`BlobSink`] is the trait the daemon's ingestion-completion path
//! fires whenever a corpus index lands. Cloud deployments wire
//! `ministr_cloud::blob_sink::BlobBackendSink`, which exports a bundle
//! and uploads to Azure Blob Storage. Self-hosted serve leaves this
//! `None` — local indexes are already durable on the user's disk.
//!
//! # Sync method, async impl
//!
//! Mirrors the [`UsageSink`] convention: fire-and-forget `enqueue`
//! method that returns immediately; the cloud impl spawns its own
//! `tokio::spawn(async { ... })` for the actual upload. Keeps the
//! trait `dyn`-safe without `Pin<Box<dyn Future>>` boilerplate.

use std::path::PathBuf;

pub trait BlobSink: Send + Sync + std::fmt::Debug {
    /// Queue an upload of the corpus at `corpus_dir` under `corpus_id`.
    /// Returns immediately; the implementation is responsible for
    /// running the export + upload off the caller's task.
    fn enqueue_upload(&self, corpus_id: String, corpus_dir: PathBuf);
}
```

### 2. `ministr-cloud/src/blob_sink.rs` (new, proprietary)

```rust
//! BlobSink impl that exports a corpus to a bundle and uploads to blob.

use std::path::PathBuf;
use std::sync::Arc;

use ministr_api::BlobSink;
use ministr_core::bundle::{self, BundleManifest};

use crate::blob_backend::BlobBackend;

#[derive(Debug, Clone)]
pub struct BlobBackendSink {
    backend: Arc<BlobBackend>,
    model_name: String,
    dimension: usize,
}

impl BlobBackendSink {
    pub fn new(backend: Arc<BlobBackend>, model_name: String, dimension: usize) -> Self {
        Self { backend, model_name, dimension }
    }
}

impl BlobSink for BlobBackendSink {
    fn enqueue_upload(&self, corpus_id: String, corpus_dir: PathBuf) {
        let backend = Arc::clone(&self.backend);
        let model_name = self.model_name.clone();
        let dimension = self.dimension;
        tokio::spawn(async move {
            // Build the manifest the same way daemon::export_bundle does.
            let mut manifest = BundleManifest {
                format_version: 1,
                model_name,
                dimension,
                vector_count: 0,
                document_count: 0,
                symbol_count: 0,
                corpus_roots: vec![],
                created_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                bundle_version: None,
                source_commit: None,
            };
            // upload_corpus calls bundle::export_bundle internally, which
            // populates corpus_roots + bundle_version from corpus_dir.
            match backend.upload_corpus(&corpus_id, &corpus_dir, &manifest).await {
                Ok(version) => tracing::info!(
                    corpus_id = %corpus_id,
                    version = %version,
                    "uploaded corpus bundle to blob"
                ),
                Err(e) => tracing::warn!(
                    corpus_id = %corpus_id,
                    error = %e,
                    "blob upload failed — corpus state will be ephemeral until next ingest"
                ),
            }
        });
    }
}
```

### 3. `ministr-daemon/src/state.rs`

Add field + chainable constructor (mirrors `with_usage_sink`):

```rust
pub blob_sink: Option<Arc<dyn BlobSink>>,

pub fn with_blob_sink(mut self, sink: Arc<dyn BlobSink>) -> Self {
    self.blob_sink = Some(sink);
    self
}
```

### 4. `ministr-daemon/src/registry.rs` — completion hook

This is the load-bearing change. The registry's per-corpus ingestion
task currently runs without an external observer. Add an
mpsc::UnboundedSender<String> that fires when a corpus's ingestion
completes successfully:

```rust
pub struct CorpusRegistry {
    // ... existing fields ...
    completion_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
}

impl CorpusRegistry {
    pub fn with_completion_channel(
        mut self,
    ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.completion_tx = Some(tx);
        (self, rx)
    }

    /// Called from the ingestion task at the success exit point.
    fn notify_complete(&self, corpus_id: &str) {
        if let Some(tx) = &self.completion_tx {
            let _ = tx.send(corpus_id.to_string());
        }
    }
}
```

Find the existing ingestion-success path (probably in `register` and
`update_corpus_paths`), call `self.notify_complete(&corpus_id)` after
the IngestionPhase=Finalizing transitions to "done".

### 5. `ministr-cli/src/commands.rs::cmd_serve_http` — wire it up

```rust
// Build the blob backend from env (existing).
let blob_backend = ministr_cloud::blob_backend::build_from_env()?;

// Boot-time download: pull every blob bundle into local /data BEFORE
// corpus_registry.restore() scans it. Skips if no backend configured.
if let Some(ref backend) = blob_backend {
    let corpus_root = config.data_dir.join("corpora");
    tokio::fs::create_dir_all(&corpus_root).await.ok();
    match backend.list_corpora().await {
        Ok(ids) => {
            tracing::info!(count = ids.len(), "restoring corpora from blob");
            for id in ids {
                let target = corpus_root.join(&id);
                tokio::fs::create_dir_all(&target).await.ok();
                if let Err(e) = backend.download_corpus(&id, &target).await {
                    tracing::warn!(
                        corpus_id = %id,
                        error = %e,
                        "blob download failed — corpus will be missing until re-indexed"
                    );
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "blob list_corpora failed at boot"),
    }
}

// Build the registry with a completion channel.
let (registry, mut completion_rx) = infra::build_corpus_registry(&ctx, config)
    .with_completion_channel();
let registry = Arc::new(registry);
registry.restore().await;

// Wire the blob sink (cloud mode) and spawn the upload reactor.
if let Some(backend) = blob_backend {
    let backend = Arc::new(backend);
    let sink: Arc<dyn BlobSink> = Arc::new(BlobBackendSink::new(
        Arc::clone(&backend),
        resolved_model.to_string(),
        resolved_dimension.unwrap_or(384),
    ));
    daemon_state = daemon_state.with_blob_sink(Arc::clone(&sink));

    // Reactor: drain completion events from the registry and dispatch
    // uploads. Single task per pod, serial uploads (one corpus's bundle
    // at a time) so the embedding model isn't competing for fs I/O.
    let registry_for_task = Arc::clone(&registry);
    tokio::spawn(async move {
        while let Some(corpus_id) = completion_rx.recv().await {
            let corpus_dir = registry_for_task.config().data_dir
                .join("corpora").join(&corpus_id);
            sink.enqueue_upload(corpus_id, corpus_dir);
        }
    });
    tracing::info!("blob durability wired — uploads after ingest, downloads at boot");
}
```

## Testing strategy

- **Unit**: mock `BlobSink` records `enqueue_upload` calls; assert the
  registry fires one per successful ingestion.
- **Integration**: spin up a `FilesystemBlobStore` against a tempdir;
  ingest a small corpus; pod-restart simulation (drop the registry,
  rebuild from same data_dir + blob); assert restored corpus has the
  same `files_indexed` as before.
- **Cloud smoke**: `just demo-remote` should still work; `az containerapp
  revision restart` then re-run demo without re-cloning; assert the
  anyhow corpus is still queryable.

## Out of scope

- **Multi-pod write coordination** — current design is single-replica.
  Multi-pod needs a lease in Postgres before upload.
- **Incremental bundle uploads** — each upload is a full bundle. Fine
  at <100 MB corpora; revisit when atlas (5K-repo cron) lands.
- **Cross-region** — blob is LRS in eastus. Geo-redundancy is a
  separate cost decision.

## Effort estimate

- BlobSink trait + impl: ~80 LoC across 2 files (1 hr).
- CorpusRegistry completion channel: ~30 LoC + finding the success
  exit point (1 hr if the path is obvious, 3 hrs if not).
- cmd_serve_http boot + reactor: ~50 LoC (45 min).
- Tests: ~200 LoC (2 hrs).
- Image rebuild + deploy + smoke test: 30 min.

Total: **4–7 focused hours** depending on how knotty the registry's
ingestion-task lifetime turns out to be.
