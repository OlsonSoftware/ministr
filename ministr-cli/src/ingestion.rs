//! Corpus ingestion orchestration for the ministr CLI.
//!
//! Contains [`run_corpus_ingestion`] which classifies corpus paths (local, web, git)
//! and dispatches to the appropriate ingestion pipeline, plus [`spawn_coherence`] and
//! [`spawn_config_watcher`] for live file watching.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use miette::{IntoDiagnostic, Result, WrapErr};

use ministr_core::coherence::{CoherenceEngine, FileWatcher};
use ministr_core::storage::Storage as _;

use crate::infra::InfrastructureContext;

/// Classify corpus paths and run the appropriate ingestion pipeline for each source type.
///
/// - Local paths are ingested via the standard file ingestion pipeline.
/// - Web URLs are fetched and ingested via `WebFetcher`.
/// - Git URLs are cloned and their content is ingested as local files.
///
/// `streaming_persist_every: Some(n)` flips the pipeline into per-batch
/// HNSW persistence mode — the index is flushed to `ctx.index_dir`
/// every `n` files indexed (PHASE4 chunk 4 plumbing). Cloud-worker
/// callers opt in because they run on a 4 GiB pod and need to bound
/// peak HNSW rss; local `ministr index` passes `None` and keeps the
/// bundle-at-end shape, avoiding gratuitous fsyncs on developer disks.
/// Build the corpus ingestion pipeline from the resolved per-corpus config.
///
/// parity-meta-toml-load: applies the SAME knobs the daemon's `indexer::run`
/// applies (parser + `min_section_tokens` + ignore patterns), so the CLI
/// one-shot and the registry ingest surfaces honor them identically.
/// rq-matryoshka-dual-ingest: when a per-corpus `dimension` is configured,
/// also stores full-dimension vectors alongside the truncated HNSW vectors so
/// query-time two-stage rerank (`with_matryoshka_rerank`) has data to rescore
/// against — without this the dimension knob was silently half-dead.
fn build_corpus_pipeline(
    ctx: &InfrastructureContext,
    progress: &Arc<ministr_core::ingestion::IngestionProgress>,
    streaming_persist_every: Option<usize>,
) -> ministr_core::ingestion::IngestionPipeline {
    let mut pipeline = ministr_core::ingestion::IngestionPipeline::new()
        .with_progress(Arc::clone(progress))
        .with_parser_override(ctx.parser)
        .with_min_section_tokens(ctx.min_section_tokens)
        .with_ignore_patterns(ctx.ignore.clone());
    if let Some(dual) = &ctx.dual_embedder {
        pipeline = pipeline.with_dual_embedder(Arc::clone(dual), (*ctx.storage).clone());
    }
    if let Some(n) = streaming_persist_every {
        pipeline = pipeline
            .with_corpus_dir(ctx.index_dir.clone())
            .with_batch_config(ministr_core::ingestion::BatchIngestionConfig {
                batch_size: 4,
                persist_every: Some(n),
            });
    }
    pipeline
}

pub(crate) async fn run_corpus_ingestion(
    corpus_paths: &[String],
    git_includes: &[ministr_core::config::GitInclude],
    ctx: &InfrastructureContext,
    progress: &Arc<ministr_core::ingestion::IngestionProgress>,
    streaming_persist_every: Option<usize>,
) -> Result<()> {
    use ministr_core::config::{CorpusSource, classify_corpus_path};

    let mut local_paths = Vec::new();
    let mut web_urls = Vec::new();
    let mut git_urls = Vec::new();

    for raw in corpus_paths {
        match classify_corpus_path(raw) {
            CorpusSource::Local(path) => local_paths.push(path),
            CorpusSource::Web(url) => web_urls.push(url),
            CorpusSource::Git(url) => git_urls.push(url),
        }
    }

    tracing::info!(
        local = local_paths.len(),
        web = web_urls.len(),
        git = git_urls.len(),
        local_paths = ?local_paths,
        streaming_persist_every,
        "classified corpus sources"
    );

    let storage = &*ctx.storage;
    let embedder = &*ctx.embedder;
    let index = &*ctx.index;

    let start = std::time::Instant::now();
    let pipeline = build_corpus_pipeline(ctx, progress, streaming_persist_every);

    // Ingest local paths.
    if !local_paths.is_empty() {
        let stats = pipeline
            .ingest_paths_with_embeddings(&local_paths, storage, embedder, index)
            .await
            .into_diagnostic()
            .wrap_err("local ingestion failed")?;

        tracing::info!(
            files_discovered = stats.files_discovered,
            files_indexed = stats.files_indexed,
            files_skipped = stats.files_skipped,
            files_removed = stats.files_removed,
            files_failed = stats.files_failed,
            sections = stats.total_sections,
            claims = stats.total_claims,
            embeddings = stats.total_embeddings,
            "local ingestion complete"
        );

        if stats.files_discovered == 0 {
            tracing::warn!(
                paths = ?local_paths,
                "no files discovered from local corpus paths — check that paths exist and contain supported files"
            );
        }
    }

    // Fetch and ingest web URLs.
    if !web_urls.is_empty() {
        ingest_web_sources(
            &web_urls,
            &ctx.corpus_dir,
            &pipeline,
            storage,
            embedder,
            index,
        )
        .await?;
    }

    // Clone and ingest git repositories (from --corpus args and .ministr.toml).
    if !git_urls.is_empty() {
        ingest_git_sources(&git_urls, &pipeline, storage, embedder, index).await;
    }
    if !git_includes.is_empty() {
        ingest_git_includes(git_includes, &pipeline, storage, embedder, index).await;
    }

    // HNSW's tar dump fails on an empty index (`Hnsw nb point 0` →
    // rename trap), so skip the persist when nothing landed. This is
    // the worker-side guard for chunk-4 enqueue flows where the
    // requested paths can legitimately be empty (e.g. the demo's
    // parent-corpus placeholder pointing at an empty `/data/corpus`).
    // The query path reads via `index.len()` on the in-memory handle,
    // so skipping the on-disk dump has no behavioural effect — the
    // next ingest that adds vectors persists normally.
    if index.is_empty() {
        tracing::info!(
            index_dir = %ctx.index_dir.display(),
            "no vectors to persist — skipping HNSW dump"
        );
    } else {
        index
            .persist(&ctx.index_dir)
            .into_diagnostic()
            .wrap_err("failed to persist vector index")?;
    }

    let elapsed_ms = crate::infra::elapsed_millis(start);
    tracing::info!(
        local = local_paths.len(),
        web = web_urls.len(),
        git = git_urls.len(),
        elapsed_ms,
        "corpus ingestion complete"
    );

    Ok(())
}

/// Fetch and ingest web URLs via `WebFetcher`.
async fn ingest_web_sources(
    urls: &[String],
    corpus_dir: &Path,
    pipeline: &ministr_core::ingestion::IngestionPipeline,
    storage: &ministr_core::storage::SqliteStorage,
    embedder: &dyn ministr_core::embedding::Embedder,
    index: &dyn ministr_core::index::VectorIndex,
) -> Result<()> {
    let web_cache_dir = corpus_dir.join("web");
    let http_client = ministr_core::web::HttpClient::with_defaults()
        .into_diagnostic()
        .wrap_err("failed to create HTTP client for corpus web fetch")?;
    let web_fetcher = ministr_core::web::fetcher::WebFetcher::new(
        http_client,
        &web_cache_dir,
        ministr_core::web::fetcher::WebFetcherConfig::default(),
    );

    for url in urls {
        match web_fetcher
            .fetch_and_ingest_with_embeddings(url, pipeline, storage, embedder, index, None)
            .await
        {
            Ok(result) => {
                tracing::info!(
                    url = %url,
                    pages = result.pages_fetched(),
                    sections = result.sections_indexed,
                    strategy = %result.strategy,
                    "web corpus ingestion complete"
                );
            }
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "web corpus ingestion failed");
            }
        }
    }
    Ok(())
}

/// Clone and ingest git repositories via `GitFetcher`.
async fn ingest_git_sources(
    urls: &[String],
    pipeline: &ministr_core::ingestion::IngestionPipeline,
    storage: &ministr_core::storage::SqliteStorage,
    embedder: &dyn ministr_core::embedding::Embedder,
    index: &dyn ministr_core::index::VectorIndex,
) {
    let git_fetcher = ministr_core::git::GitFetcher::with_defaults();

    for url in urls {
        match git_fetcher.clone(url, None, None, None).await {
            Ok(clone_result) => {
                // Register a corpus root for the clone so it persists across sessions.
                let root_id = ministr_core::ingestion::compute_root_id(&clone_result.clone_dir);
                let clone_root = ministr_core::types::CorpusRoot {
                    id: root_id.clone(),
                    path: clone_result.clone_dir.to_string_lossy().to_string(),
                    kind: ministr_core::types::RootKind::Git,
                    display_name: Some(git_repo_display_name(url)),
                    file_count: 0,
                    language_stats: std::collections::HashMap::new(),
                    repo_url: Some(url.clone()),
                    branch: clone_result.metadata.branch.clone(),
                    commit_sha: Some(clone_result.metadata.commit_sha.clone()),
                    clone_timestamp: Some(clone_result.metadata.clone_timestamp.clone()),
                    sparse_paths: clone_result.metadata.checked_out_paths.clone(),
                };
                if let Err(e) = storage.upsert_corpus_root(&clone_root).await {
                    tracing::warn!(
                        url = %url,
                        error = %e,
                        "failed to register clone corpus root"
                    );
                }

                // Ingest with root-scoped ingestion to namespace documents.
                match pipeline
                    .ingest_directory_with_embeddings_rooted(
                        &clone_result.clone_dir,
                        storage,
                        embedder,
                        index,
                        Some(&root_id),
                        None,
                    )
                    .await
                {
                    Ok(stats) => {
                        // Update the root's file count after ingestion.
                        let updated_root = ministr_core::types::CorpusRoot {
                            file_count: stats.files_indexed,
                            ..clone_root
                        };
                        if let Err(e) = storage.upsert_corpus_root(&updated_root).await {
                            tracing::warn!(
                                url = %url,
                                error = %e,
                                "failed to update clone root stats"
                            );
                        }

                        // Record in git cache for staleness tracking.
                        let git_cache_record = ministr_core::storage::GitCacheRecord {
                            repo_url: url.clone(),
                            branch: clone_result.metadata.branch.clone(),
                            commit_sha: clone_result.metadata.commit_sha.clone(),
                            clone_timestamp: clone_result.metadata.clone_timestamp.clone(),
                            clone_dir: clone_result.clone_dir.to_string_lossy().to_string(),
                            checked_out_paths: clone_result.metadata.checked_out_paths.clone(),
                        };
                        if let Err(e) = storage.upsert_git_cache(&git_cache_record).await {
                            tracing::warn!(
                                url = %url,
                                error = %e,
                                "failed to record git cache"
                            );
                        }

                        tracing::info!(
                            url = %url,
                            clone_dir = %clone_result.clone_dir.display(),
                            files_indexed = stats.files_indexed,
                            sections = stats.total_sections,
                            root_id = %root_id,
                            "git corpus ingestion complete"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            url = %url,
                            error = %e,
                            "git corpus file ingestion failed"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "git corpus clone failed");
            }
        }
    }
}

/// Clone and ingest git repositories specified in `.ministr.toml`.
///
/// Unlike [`ingest_git_sources`], this accepts [`GitInclude`] structs
/// which support sparse checkout paths and branch selection.
async fn ingest_git_includes(
    includes: &[ministr_core::config::GitInclude],
    pipeline: &ministr_core::ingestion::IngestionPipeline,
    storage: &ministr_core::storage::SqliteStorage,
    embedder: &dyn ministr_core::embedding::Embedder,
    index: &dyn ministr_core::index::VectorIndex,
) {
    let git_fetcher = ministr_core::git::GitFetcher::with_defaults();

    for inc in includes {
        let paths_ref: Option<Vec<String>> = inc.paths.clone();
        match git_fetcher
            .clone(&inc.repo, paths_ref.as_deref(), inc.branch.as_deref(), None)
            .await
        {
            Ok(clone_result) => {
                let root_id = ministr_core::ingestion::compute_root_id(&clone_result.clone_dir);
                let clone_root = ministr_core::types::CorpusRoot {
                    id: root_id.clone(),
                    path: clone_result.clone_dir.to_string_lossy().to_string(),
                    kind: ministr_core::types::RootKind::Git,
                    display_name: Some(git_repo_display_name(&inc.repo)),
                    file_count: 0,
                    language_stats: std::collections::HashMap::new(),
                    repo_url: Some(inc.repo.clone()),
                    branch: clone_result.metadata.branch.clone(),
                    commit_sha: Some(clone_result.metadata.commit_sha.clone()),
                    clone_timestamp: Some(clone_result.metadata.clone_timestamp.clone()),
                    sparse_paths: clone_result.metadata.checked_out_paths.clone(),
                };
                if let Err(e) = storage.upsert_corpus_root(&clone_root).await {
                    tracing::warn!(repo = %inc.repo, error = %e, "failed to register clone root");
                }

                match pipeline
                    .ingest_directory_with_embeddings_rooted(
                        &clone_result.clone_dir,
                        storage,
                        embedder,
                        index,
                        Some(&root_id),
                        None,
                    )
                    .await
                {
                    Ok(stats) => {
                        let updated_root = ministr_core::types::CorpusRoot {
                            file_count: stats.files_indexed,
                            ..clone_root
                        };
                        let _ = storage.upsert_corpus_root(&updated_root).await;

                        let git_cache_record = ministr_core::storage::GitCacheRecord {
                            repo_url: inc.repo.clone(),
                            branch: clone_result.metadata.branch.clone(),
                            commit_sha: clone_result.metadata.commit_sha.clone(),
                            clone_timestamp: clone_result.metadata.clone_timestamp.clone(),
                            clone_dir: clone_result.clone_dir.to_string_lossy().to_string(),
                            checked_out_paths: clone_result.metadata.checked_out_paths.clone(),
                        };
                        let _ = storage.upsert_git_cache(&git_cache_record).await;

                        tracing::info!(
                            repo = %inc.repo,
                            files_indexed = stats.files_indexed,
                            sections = stats.total_sections,
                            "git include from .ministr.toml ingested"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(repo = %inc.repo, error = %e, "git include ingestion failed");
                    }
                }
            }
            Err(e) => {
                tracing::warn!(repo = %inc.repo, error = %e, "git include clone failed");
            }
        }
    }
}

/// Derive a human-readable display name from a git repository URL.
///
/// Extracts the `owner/repo` portion from common URL formats, falling
/// back to the full URL if parsing fails.
fn git_repo_display_name(url: &str) -> String {
    // Strip trailing .git
    let cleaned = url.strip_suffix(".git").unwrap_or(url);
    // Try to extract owner/repo from the last two path segments.
    let segments: Vec<&str> = cleaned.rsplit('/').take(2).collect();
    if segments.len() == 2 {
        format!("{}/{}", segments[1], segments[0])
    } else {
        cleaned.to_string()
    }
}

/// Spawn the coherence file watcher and background processing task.
///
/// Watches all corpus paths for file changes, re-indexes affected files
/// (including embeddings and vector index), and propagates coherence alerts
/// to the active session.
pub(crate) fn spawn_coherence(
    corpus_paths: &[PathBuf],
    server: &ministr_mcp::server::MinistrServer,
    storage: &Arc<ministr_core::storage::SqliteStorage>,
    embedder: &Arc<dyn ministr_core::embedding::Embedder>,
    index: &Arc<dyn ministr_core::index::VectorIndex>,
    ignore_patterns: &[String],
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    // Collect watch paths: directories directly, individual files via their parent.
    let watch_paths: Vec<PathBuf> = corpus_paths
        .iter()
        .map(|p| {
            if p.is_dir() {
                p.clone()
            } else {
                p.parent().unwrap_or(p).to_path_buf()
            }
        })
        .collect();

    let watcher = FileWatcher::new(&watch_paths)
        .into_diagnostic()
        .wrap_err("failed to start file watcher for coherence")?;

    // Use the first directory path as the primary corpus_dir for the coherence engine.
    let primary_dir = corpus_paths
        .iter()
        .find(|p| p.is_dir())
        .cloned()
        .or_else(|| {
            corpus_paths
                .first()
                .and_then(|p| p.parent().map(Path::to_path_buf))
        })
        .unwrap_or_else(|| PathBuf::from("."));

    let engine = Arc::new(
        CoherenceEngine::with_embeddings(primary_dir, Arc::clone(embedder), Arc::clone(index))
            // watcher-ignore-filtering: excluded paths never drive a
            // per-file reindex on this surface.
            .with_exclusions(&watch_paths, ignore_patterns),
    );

    let registry = server.registry_arc();

    // Create a channel for pushing coherence change notifications to MCP
    // resource subscribers (e.g. ministr://status).
    let (notify_tx, notify_rx) = tokio::sync::mpsc::unbounded_channel();
    server.set_coherence_receiver(notify_rx);

    let handle = ministr_core::coherence::spawn_coherence_task(
        watcher,
        engine,
        Arc::clone(storage),
        registry,
        Some(notify_tx),
    );

    tracing::info!(
        corpus = ?corpus_paths,
        "coherence file watcher started"
    );

    Ok(Some(handle))
}

/// Spawn a background task that watches `.ministr.toml` for changes and re-indexes
/// new corpus paths automatically.
///
/// When the config file is modified, the watcher:
/// 1. Re-reads and re-resolves corpus paths from `.ministr.toml`
/// 2. Diffs against the set of paths that were indexed at startup
/// 3. Runs ingestion for any newly added paths
///
/// This lets users add paths to `.ministr.toml` without restarting the MCP session.
#[allow(clippy::too_many_lines)] // config watcher is a single coherent setup block
pub(crate) fn spawn_config_watcher(
    config_dir: PathBuf,
    initial_paths: Vec<String>,
    ctx: &InfrastructureContext,
    ingestion_progress: &Arc<ministr_core::ingestion::IngestionProgress>,
) -> Option<tokio::task::JoinHandle<()>> {
    use notify::{RecursiveMode, Watcher};

    let config_file = config_dir.join(ministr_core::config::CORPUS_CONFIG_FILENAME);
    if !config_file.exists() {
        return None;
    }

    // Use a raw notify watcher (not FileWatcher) because FileWatcher filters
    // by indexable file types and would silently drop .toml events.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<PathBuf>(32);
    let config_filename = std::ffi::OsString::from(ministr_core::config::CORPUS_CONFIG_FILENAME);

    let mut watcher =
        match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                // Only forward events that touch .ministr.toml.
                for path in &event.paths {
                    if path.file_name() == Some(&config_filename) {
                        let _ = tx.try_send(path.clone());
                        break;
                    }
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to watch .ministr.toml — config hot-reload disabled"
                );
                return None;
            }
        };

    // Watch the config directory non-recursively (just need .ministr.toml changes).
    if let Err(e) = watcher.watch(&config_dir, RecursiveMode::NonRecursive) {
        tracing::warn!(
            error = %e,
            "failed to watch config directory — config hot-reload disabled"
        );
        return None;
    }

    let bg_ctx = ctx.clone();
    let progress = Arc::clone(ingestion_progress);

    tracing::info!(
        config = %config_file.display(),
        "watching .ministr.toml for path changes"
    );

    let handle = tokio::spawn(async move {
        // Keep the watcher alive for the lifetime of this task.
        let _watcher = watcher;
        let mut known_paths: std::collections::HashSet<String> =
            initial_paths.into_iter().collect();

        while rx.recv().await.is_some() {
            // Debounce: editors often write tmp files then rename.
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            while rx.try_recv().is_ok() {}

            tracing::info!("detected .ministr.toml change — checking for new corpus paths");

            // Re-read the config.
            let repo_config: ministr_core::config::RepoConfig =
                match std::fs::read_to_string(&config_file) {
                    Ok(contents) => match toml::from_str(&contents) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to parse .ministr.toml");
                            continue;
                        }
                    },
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to read .ministr.toml");
                        continue;
                    }
                };

            let fresh_paths: std::collections::HashSet<String> = repo_config
                .resolve_local_paths(&config_dir)
                .into_iter()
                .collect();

            let new_paths: Vec<String> = fresh_paths.difference(&known_paths).cloned().collect();

            if new_paths.is_empty() {
                tracing::info!("no new corpus paths found");
                continue;
            }

            tracing::info!(
                new_count = new_paths.len(),
                paths = ?new_paths,
                "new corpus paths detected — starting ingestion"
            );

            let git_includes = repo_config.corpus.git.clone();

            match run_corpus_ingestion(&new_paths, &git_includes, &bg_ctx, &progress, None).await {
                Ok(()) => {
                    known_paths.extend(new_paths);
                    tracing::info!("config-triggered ingestion complete");
                }
                Err(e) => {
                    tracing::error!(error = %e, "config-triggered ingestion failed");
                }
            }
        }
    });

    Some(handle)
}
