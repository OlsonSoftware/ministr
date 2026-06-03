//! Shared infrastructure setup for the ministr CLI.
//!
//! Initializes storage, embedder, vector index, and the MCP server. These
//! components are shared across all `serve` and `index` subcommands.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use miette::{IntoDiagnostic, Result, WrapErr};

use sha2::Digest as _;

use ministr_core::index::VectorIndexLoad as _;
use ministr_core::session::UsageConfig;
use ministr_core::storage::Storage as _;

/// Shared infrastructure components initialized at startup.
#[derive(Clone)]
pub(crate) struct InfrastructureContext {
    pub(crate) corpus_dir: PathBuf,
    pub(crate) index_dir: PathBuf,
    pub(crate) storage: Arc<ministr_core::storage::SqliteStorage>,
    pub(crate) embedder: Arc<dyn ministr_core::embedding::Embedder>,
    pub(crate) index: Arc<dyn ministr_core::index::VectorIndex>,
    /// Dual embedder for two-stage Matryoshka retrieval (set when dimension is configured).
    pub(crate) dual_embedder: Option<Arc<dyn ministr_core::embedding::DualEmbedder>>,
    /// Number of coarse candidates to rescore with full-dim vectors.
    pub(crate) rerank_depth: usize,
    /// Per-corpus parser override resolved from `meta.toml` (parity-meta-toml-load);
    /// `None` = auto-detect by extension. Applied to the ingestion pipeline.
    pub(crate) parser: Option<ministr_core::parser::ParserKind>,
    /// Per-corpus minimum standalone-section token count resolved from `meta.toml`
    /// (parity-meta-toml-load; default 50). Applied to the ingestion pipeline.
    pub(crate) min_section_tokens: usize,
}

/// Initialize shared infrastructure: storage, embedder, and vector index.
///
/// Returns the corpus data directory, index directory, and Arc-wrapped components.
#[allow(clippy::too_many_lines)] // embedder-kind branch + cache-key logic; further splitting would obscure the local-vs-remote flow
pub(crate) async fn init_infrastructure(
    corpus_paths: &[String],
    config: &ministr_core::config::MinistrConfig,
    resolved_model: Option<&str>,
    resolved_dimension: Option<usize>,
    rerank_depth: Option<usize>,
) -> Result<InfrastructureContext> {
    // Determine corpus data directory from a hash of all paths.
    let corpus_name = if corpus_paths.is_empty() {
        "default".to_owned()
    } else {
        corpus_data_dir_name(corpus_paths)
    };

    let corpus_dir = config.data_dir.join("corpora").join(&corpus_name);
    let db_path = corpus_dir.join("content.db");

    migrate_legacy_corpus_dir(config, corpus_paths, &corpus_name, &corpus_dir);

    // Create corpus directory if it doesn't exist.
    std::fs::create_dir_all(&corpus_dir)
        .into_diagnostic()
        .wrap_err_with(|| {
            format!(
                "failed to create corpus directory: {}",
                corpus_dir.display()
            )
        })?;

    // Initialize storage.
    let storage = ministr_core::storage::SqliteStorage::open(&db_path)
        .into_diagnostic()
        .wrap_err("failed to open content database")?;

    // Use resolved model name or fall back to global default.
    let model_name = resolved_model.map_or_else(|| config.default_model.clone(), String::from);
    tracing::info!(model = %model_name, "resolved embedding model");

    // PHASE6 chunk 2 — embedder selection.
    //
    // When `MINISTR_EMBEDDER_KIND=openai` is set AND the Azure `OpenAI`
    // env (endpoint, deployment, auth) resolves, build an
    // [`ministr_core::embedding::openai::OpenAiEmbedder`] and SKIP the local fastembed
    // model load entirely. The model file isn't downloaded, ONNX
    // runtime isn't initialised, no GPU device is touched. This is the
    // cloud worker's primary path — it drops worker pod memory from
    // ~3.6 GiB (local ONNX) to <500 MiB (just SQLite + HNSW + reqwest).
    //
    // Local CLI (`ministr index`) leaves the env var unset and gets
    // the existing fastembed/ONNX path unchanged.
    //
    // Failure-mode: env var set to "openai" but config incomplete →
    // hard error (don't silently fall back to the local 3.6 GiB path on
    // a cloud pod that may not be sized for it).
    let embedder_kind = std::env::var("MINISTR_EMBEDDER_KIND").ok();
    let use_openai = embedder_kind.as_deref() == Some("openai");

    let (embedder, dual_embedder, cache_model_key): (
        Arc<dyn ministr_core::embedding::Embedder>,
        Option<Arc<dyn ministr_core::embedding::DualEmbedder>>,
        String,
    ) = if use_openai {
        tracing::info!("MINISTR_EMBEDDER_KIND=openai — using Azure OpenAI embedder");
        let openai_cfg =
            ministr_core::embedding::openai::OpenAiConfig::from_env().ok_or_else(|| {
                miette::miette!(
                    "MINISTR_EMBEDDER_KIND=openai requires MINISTR_AZURE_OPENAI_ENDPOINT, \
                 MINISTR_AZURE_OPENAI_DEPLOYMENT, and an auth source \
                 (MINISTR_AZURE_OPENAI_API_KEY or IDENTITY_ENDPOINT+IDENTITY_HEADER)",
                )
            })?;
        // Honour resolved_dimension when set; otherwise default to 384
        // (`ministr_core::embedding::openai::DEFAULT_DIMENSIONS`) so HNSW indexes stay
        // cross-compatible with the local fastembed family.
        let dim = resolved_dimension.unwrap_or(ministr_core::embedding::openai::DEFAULT_DIMENSIONS);
        let remote =
            ministr_core::embedding::openai::OpenAiEmbedder::with_dimensions(openai_cfg, dim)
                .map_err(|e| miette::miette!("build OpenAiEmbedder: {e}"))?;
        let arc: Arc<dyn ministr_core::embedding::Embedder> = Arc::new(remote);
        // No DualEmbedder for the remote path — Matryoshka is a
        // fastembed-specific facility; OpenAI v3 supports the
        // `dimensions` param natively which we already pin above.
        let key = format!("{model_name}:openai:{dim}");
        (arc, None, key)
    } else {
        // Local fastembed/ONNX path (default).
        //
        // Backend selection:
        // - MINISTR_BACKEND=candle  → Candle with Metal GPU (fastest on Apple Silicon)
        // - MINISTR_BACKEND=onnx    → FastEmbed/ONNX Runtime with CoreML (default)
        // - unset on macOS with candle feature → auto-detect: use Candle if the model
        //   is supported, otherwise fall back to ONNX.
        ministr_core::mem_profile::checkpoint("before embedding model init");
        let (raw_embedder, backend_info) = create_embedder(&model_name, &config.data_dir)?;
        tracing::info!(
            backend = ?backend_info.format,
            device = %backend_info.device,
            "embedding backend selected"
        );
        ministr_core::mem_profile::checkpoint("after embedding model init");

        // Backend-aware cache key matches the historical scheme so
        // existing SQLite caches survive the PHASE6 refactor on local
        // CLI users' boxes.
        let key = format!("{model_name}{}", backend_info.cache_key_suffix());

        // Wrap in MatryoshkaEmbedder when dimension is configured for two-stage retrieval.
        if let Some(target_dim) = resolved_dimension {
            tracing::info!(
                target_dim,
                full_dim = raw_embedder.dimension(),
                "Matryoshka two-stage retrieval enabled"
            );
            let matryoshka = Arc::new(
                ministr_core::embedding::MatryoshkaEmbedder::new(
                    Arc::clone(&raw_embedder),
                    target_dim,
                )
                .into_diagnostic()
                .wrap_err("failed to create MatryoshkaEmbedder")?,
            );
            let emb: Arc<dyn ministr_core::embedding::Embedder> = Arc::clone(&matryoshka) as _;
            let dual: Arc<dyn ministr_core::embedding::DualEmbedder> = matryoshka;
            (emb, Some(dual), key)
        } else {
            (raw_embedder, None, key)
        }
    };

    let embedding_cache = ministr_core::embedding::cache::EmbeddingCache::new(storage.conn());
    let embedder: Arc<dyn ministr_core::embedding::Embedder> = Arc::new(
        ministr_core::embedding::CachedEmbedder::new(embedder, embedding_cache, &cache_model_key),
    );

    // Initialize vector index.
    // If the SQLite DB is empty (fresh migration) but a stale vector index
    // exists on disk, discard it to avoid phantom IDs from a previous run.
    let dim = embedder.dimension();
    let index_dir = corpus_dir.join("index");
    let doc_count: usize = storage
        .document_count()
        .await
        .into_diagnostic()
        .wrap_err("failed to check document count")?;
    if doc_count == 0 && index_dir.exists() {
        tracing::warn!("empty database with stale vector index — discarding old index");
        std::fs::remove_dir_all(&index_dir)
            .into_diagnostic()
            .wrap_err("failed to remove stale vector index")?;
    }

    ministr_core::mem_profile::checkpoint("before vector index init");
    let index: Arc<dyn ministr_core::index::VectorIndex> =
        load_or_create_index(&index_dir, dim, &model_name, &storage).await?;

    ministr_core::mem_profile::checkpoint("after vector index init");

    // parity-meta-toml-load: resolve this corpus's per-corpus `meta.toml` knobs
    // (parser + min_section_tokens) through the shared seam so `ministr index`
    // honors them exactly as the daemon registry does. Absent/unparseable
    // meta.toml → defaults (None / 50). parser + min_section_tokens live only in
    // `meta.toml`, so a `None` repo config here is correct.
    let meta = ministr_core::config::CorpusConfig::load(&corpus_dir.join("meta.toml")).ok();
    let effective =
        ministr_core::config::resolve_effective_corpus_config(None, meta.as_ref(), config);

    Ok(InfrastructureContext {
        corpus_dir,
        index_dir,
        storage: Arc::new(storage),
        embedder,
        index,
        dual_embedder,
        rerank_depth: rerank_depth.unwrap_or(100),
        parser: effective.parser,
        min_section_tokens: effective.min_section_tokens,
    })
}

/// Create the raw embedding model based on backend preference.
///
/// Delegates to [`ministr_core::embedding::create_embedder`] which handles
/// `MINISTR_BACKEND`, `MINISTR_DEVICE`, and `MINISTR_PREFER_QUANTIZED` env vars.
fn create_embedder(
    model_name: &str,
    data_dir: &Path,
) -> Result<(
    Arc<dyn ministr_core::embedding::Embedder>,
    ministr_core::embedding::BackendInfo,
)> {
    ministr_core::embedding::create_embedder(model_name, data_dir)
        .into_diagnostic()
        .wrap_err("failed to initialize embedding model")
}

/// Load an existing HNSW index or create a fresh one.
///
/// Detects embedding model changes (dimension or model name mismatch) and
/// discards the old index when the model has changed, forcing a re-index.
async fn load_or_create_index(
    index_dir: &Path,
    dim: usize,
    model_name: &str,
    storage: &ministr_core::storage::SqliteStorage,
) -> Result<Arc<dyn ministr_core::index::VectorIndex>> {
    // ADR 0001 D4 + f-ingest-hnsw-cache-token (parity with the daemon): the
    // ACID `indexed_vectors` table is the source of truth; the HNSW is a
    // derived cache. On the CLI serve path we LOAD a persisted dump when a
    // validity token proves it still matches the source of truth, else
    // REBUILD from SQLite (re-running the degenerate guard) — so the
    // zero-vector-poison and "fixed in code / stale on disk" bug classes stay
    // structurally impossible and no graph can drift. `None` means a legacy
    // corpus with no `indexed_vectors` (pre-V24): fall through to the on-disk
    // dump, then a fresh index.
    match ministr_core::index::load_cached_or_rebuild_hnsw(
        storage,
        index_dir,
        dim,
        Some(model_name),
    )
    .await
    {
        Ok(Some(index)) => return Ok(Arc::new(index)),
        Ok(None) => {
            // No persisted indexed vectors — a legacy corpus. Fall through to
            // the on-disk dump so pre-V24 corpora keep loading unchanged.
        }
        Err(e) => {
            tracing::warn!(error = %e, "cache/rebuild from SQLite source of truth failed; falling back to on-disk index");
        }
    }

    if index_dir.exists() {
        match ministr_core::index::HnswIndex::load(index_dir) {
            Ok(loaded) => match loaded.check_compatible(dim, model_name, index_dir) {
                Ok(()) => {
                    // Compatible. A legacy index with no stored model
                    // name is adopted under the current model.
                    if loaded.model_name().is_none() {
                        tracing::info!(
                            model = %model_name,
                            "upgrading legacy index with model name tracking"
                        );
                        loaded.set_model_name(model_name);
                    }
                    return Ok(Arc::new(loaded));
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "embedding model changed — discarding old index for re-indexing"
                    );
                    drop(loaded);
                    discard_index_dir(index_dir);
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "corrupted vector index — discarding and rebuilding");
                discard_index_dir(index_dir);
            }
        }
    }
    create_fresh_index(dim, model_name)
}

/// Remove an index directory that is corrupt or incompatible so a fresh
/// one can be built. Uses the Windows-robust retrying remove and logs
/// (rather than silently swallowing) a removal failure — a stale dir
/// left behind would otherwise be re-loaded on the next run.
fn discard_index_dir(index_dir: &Path) {
    if let Err(e) = ministr_core::fs_util::remove_dir_all_robust_sync(index_dir) {
        tracing::warn!(
            error = %e,
            dir = %index_dir.display(),
            "failed to remove stale index directory; a rebuild will overwrite it"
        );
    }
}

/// Create a fresh HNSW index with the given dimension and model name.
fn create_fresh_index(
    dim: usize,
    model_name: &str,
) -> Result<Arc<dyn ministr_core::index::VectorIndex>> {
    let fresh = ministr_core::index::HnswIndex::new(dim, 100_000)
        .into_diagnostic()
        .wrap_err("failed to create vector index")?;
    fresh.set_model_name(model_name);
    Ok(Arc::new(fresh))
}

/// Build a fully configured `MinistrServer` with web fetcher, git fetcher, and coherence watcher.
///
/// Returns the server and a coherence handle that must be kept alive.
#[allow(clippy::too_many_lines)]
pub(crate) async fn build_server(
    corpus_paths: &[String],
    config_path: &Path,
    config: &ministr_core::config::MinistrConfig,
    resolved_model: Option<&str>,
    resolved_dimension: Option<usize>,
    rerank_depth: Option<usize>,
) -> Result<(
    ministr_mcp::server::MinistrServer,
    InfrastructureContext,
    Option<tokio::task::JoinHandle<()>>,
)> {
    tracing::info!(
        corpus_count = corpus_paths.len(),
        config = %config_path.display(),
        "ministr starting — {} corpus path(s)",
        corpus_paths.len()
    );
    for path in corpus_paths {
        tracing::info!(path = %path, "  corpus root");
    }

    let ctx = init_infrastructure(
        corpus_paths,
        config,
        resolved_model,
        resolved_dimension,
        rerank_depth,
    )
    .await?;

    let mut service = ministr_core::service::QueryService::new(
        (*ctx.storage).clone(),
        Arc::clone(&ctx.embedder),
        Arc::clone(&ctx.index),
    );
    if let Some(ref dual_emb) = ctx.dual_embedder {
        service = service.with_matryoshka_rerank(Arc::clone(dual_emb), ctx.rerank_depth);
    }
    let service = Arc::new(service);

    let session_id = corpus_session_id(corpus_paths);
    let budget_config = UsageConfig {
        max_context_tokens: config.default_context_budget,
        ..UsageConfig::default()
    };

    let server = ministr_mcp::server::MinistrServer::with_persistence(
        service,
        budget_config,
        Arc::clone(&ctx.storage),
        session_id,
    )
    .await;

    let server = enable_web_fetcher(server, &ctx.corpus_dir, &ctx.embedder, &ctx.index)?;
    let server = enable_git_fetcher(server, &ctx.embedder, &ctx.index);

    // Spawn coherence file watcher.
    let local_paths: Vec<PathBuf> = corpus_paths
        .iter()
        .filter_map(|p| {
            if let ministr_core::config::CorpusSource::Local(path) =
                ministr_core::config::classify_corpus_path(p)
            {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    let coherence_handle = if local_paths.is_empty() {
        None
    } else {
        crate::ingestion::spawn_coherence(
            &local_paths,
            &server,
            &ctx.storage,
            &ctx.embedder,
            &ctx.index,
        )?
    };

    // Prune tools that are irrelevant for this corpus configuration.
    let mut server = server;
    server.prune_tools(&local_paths);

    Ok((server, ctx, coherence_handle))
}

/// Spawn background corpus ingestion, returning when the MCP transport finishes.
pub(crate) fn spawn_background_ingestion(
    corpus_paths: &[String],
    git_includes: &[ministr_core::config::GitInclude],
    ctx: &InfrastructureContext,
    ingestion_progress: &Arc<ministr_core::ingestion::IngestionProgress>,
) {
    if corpus_paths.is_empty() && git_includes.is_empty() {
        return;
    }
    let bg_corpus_paths = corpus_paths.to_vec();
    let bg_git_includes = git_includes.to_vec();
    let bg_ctx = ctx.clone();
    let bg_progress = Arc::clone(ingestion_progress);
    tokio::spawn(async move {
        match crate::ingestion::run_corpus_ingestion(
            &bg_corpus_paths,
            &bg_git_includes,
            &bg_ctx,
            &bg_progress,
            None, // local/dev path — bundle at end, no mid-run persist
        )
        .await
        {
            Ok(()) => tracing::info!("background corpus ingestion complete"),
            Err(e) => {
                tracing::error!(error = %e, "background corpus ingestion failed");
                bg_progress.complete();
            }
        }
    });
}

/// Enable web fetching on the server by constructing an `HttpClient` and `WebFetcher`.
fn enable_web_fetcher(
    server: ministr_mcp::server::MinistrServer,
    corpus_dir: &Path,
    embedder: &Arc<dyn ministr_core::embedding::Embedder>,
    index: &Arc<dyn ministr_core::index::VectorIndex>,
) -> Result<ministr_mcp::server::MinistrServer> {
    let web_cache_dir = corpus_dir.join("web");
    let http_client = ministr_core::web::HttpClient::with_defaults()
        .into_diagnostic()
        .wrap_err("failed to create HTTP client for web fetcher")?;
    let web_fetcher = ministr_core::web::fetcher::WebFetcher::new(
        http_client,
        &web_cache_dir,
        ministr_core::web::fetcher::WebFetcherConfig::default(),
    );
    Ok(server.with_web_fetcher(web_fetcher, Arc::clone(embedder), Arc::clone(index)))
}

/// Enable git cloning on the server by constructing a `GitFetcher`.
fn enable_git_fetcher(
    server: ministr_mcp::server::MinistrServer,
    embedder: &Arc<dyn ministr_core::embedding::Embedder>,
    index: &Arc<dyn ministr_core::index::VectorIndex>,
) -> ministr_mcp::server::MinistrServer {
    let git_fetcher = ministr_core::git::GitFetcher::with_defaults();
    server.with_git_fetcher(git_fetcher, Arc::clone(embedder), Arc::clone(index))
}

/// Derive a stable session ID from the corpus paths so sessions persist across restarts.
pub(crate) fn corpus_session_id(corpus_paths: &[String]) -> Option<String> {
    if corpus_paths.is_empty() {
        return None;
    }
    let mut hasher = sha2::Sha256::new();
    for p in corpus_paths {
        sha2::Digest::update(&mut hasher, p.as_bytes());
        sha2::Digest::update(&mut hasher, b"\0");
    }
    let hash = sha2::Digest::finalize(hasher);
    Some(format!(
        "ministr-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7]
    ))
}

/// Derive a stable data directory name from corpus paths.
///
/// Delegates to [`ministr_core::corpus_id::corpus_id_from_paths`] — the
/// single source of truth shared with the daemon's registry. The CLI and
/// daemon both resolve a corpus to `<data_dir>/corpora/<this name>`, so
/// they MUST agree: a divergence silently splits one project's index
/// across two directories (data loss with no error). Empty / invalid
/// path sets fall back to `"default"`, matching the caller's own
/// empty-slice handling in [`init_infrastructure`].
pub(crate) fn corpus_data_dir_name(corpus_paths: &[String]) -> String {
    ministr_core::corpus_id::corpus_id_from_paths(corpus_paths)
        .unwrap_or_else(|_| "default".to_owned())
}

/// One-time migration from the pre-unification directory scheme.
///
/// If the canonical dir doesn't exist yet but an older CLI already
/// indexed this project under the legacy name, move it across so we
/// reuse the existing index instead of silently re-indexing from
/// scratch. Best-effort: a collision (both dirs present) or rename
/// failure just leaves the legacy dir untouched — never overwrite
/// live data.
fn migrate_legacy_corpus_dir(
    config: &ministr_core::config::MinistrConfig,
    corpus_paths: &[String],
    corpus_name: &str,
    corpus_dir: &Path,
) {
    if corpus_paths.is_empty() || corpus_dir.exists() {
        return;
    }
    let legacy_name = legacy_corpus_data_dir_name(corpus_paths);
    if legacy_name == corpus_name {
        return;
    }
    let legacy_dir = config.data_dir.join("corpora").join(&legacy_name);
    if !legacy_dir.is_dir() {
        return;
    }
    match std::fs::rename(&legacy_dir, corpus_dir) {
        Ok(()) => tracing::info!(
            legacy = %legacy_dir.display(),
            canonical = %corpus_dir.display(),
            "migrated corpus dir to canonical id"
        ),
        Err(e) => tracing::warn!(
            error = %e,
            legacy = %legacy_dir.display(),
            "failed to migrate legacy corpus dir; re-indexing under canonical id"
        ),
    }
}

/// The pre-unification directory-name scheme.
///
/// Retained solely so [`init_infrastructure`] can migrate an existing
/// on-disk corpus (indexed by an older CLI) to the canonical name instead
/// of orphaning it and silently re-indexing from scratch.
fn legacy_corpus_data_dir_name(corpus_paths: &[String]) -> String {
    if corpus_paths.len() == 1 {
        let p = &corpus_paths[0];
        let name = p.rsplit('/').find(|s| !s.is_empty()).unwrap_or(p);
        if !name.contains("://") && !name.contains(':') {
            return name.to_owned();
        }
    }
    let mut hasher = sha2::Sha256::new();
    for p in corpus_paths {
        sha2::Digest::update(&mut hasher, p.as_bytes());
        sha2::Digest::update(&mut hasher, b"\0");
    }
    let hash = sha2::Digest::finalize(hasher);
    format!(
        "multi-{:02x}{:02x}{:02x}{:02x}",
        hash[0], hash[1], hash[2], hash[3]
    )
}

/// Convert elapsed duration to milliseconds, saturating at `u64::MAX`.
pub(crate) fn elapsed_millis(start: std::time::Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Build a `CorpusRegistry` that shares the same `Arc<dyn Embedder>` as the
/// MCP server's `InfrastructureContext`. Used by `cmd_serve_http` to expose
/// `ministr-daemon::daemon::*_router` REST routes alongside the MCP routes
/// without spinning up a second embedding model.
pub(crate) fn build_corpus_registry(
    ctx: &InfrastructureContext,
    config: &ministr_core::config::MinistrConfig,
) -> Arc<ministr_daemon::registry::CorpusRegistry> {
    Arc::new(ministr_daemon::registry::CorpusRegistry::new(
        Arc::clone(&ctx.embedder),
        config.clone(),
    ))
}
