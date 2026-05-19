//! Refresh and clone-and-ingest pipeline helpers for the ministr server.
//!
//! These `impl MinistrServer` methods handle the `ministr_refresh` and `ministr_clone`
//! tool logic — checking cached web/git sources for staleness, re-fetching
//! changed content, and running the ingestion pipeline on cloned repositories.

use std::sync::Arc;

use rmcp::model::{CallToolResult, Content, ErrorData as McpError};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use ministr_core::code::package_graph::PackageGraph;
use ministr_core::embedding::Embedder;
use ministr_core::git::GitFetcher;
use ministr_core::index::VectorIndex;
use ministr_core::storage::{SqliteStorage, Storage};

use super::MinistrServer;
use super::helpers::{
    compute_language_stats, elapsed_millis, repo_display_name, structured_result,
};
use super::types::{
    CloneParams, CloneResponse, RefreshGitDetailResponse, RefreshParams, RefreshResponse,
    RefreshUrlDetailResponse,
};

impl MinistrServer {
    /// Execute the clone-and-ingest pipeline for `ministr_clone`.
    ///
    /// Separated from the tool handler to satisfy the `too_many_lines` lint.
    #[allow(clippy::too_many_lines)]
    pub(super) async fn clone_and_ingest(
        &self,
        params: &CloneParams,
        git_fetcher: &GitFetcher,
        embedder: &dyn Embedder,
        index: &dyn VectorIndex,
        storage: &SqliteStorage,
        ct: Option<&CancellationToken>,
    ) -> Result<CallToolResult, McpError> {
        // Phase 1: Clone the repository.
        let clone_start = std::time::Instant::now();
        let clone_result = match GitFetcher::clone(
            git_fetcher,
            &params.repo,
            params.paths.as_deref(),
            params.branch.as_deref(),
            ct,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, repo = %params.repo, "ministr_clone: git clone failed");
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "clone failed: {e}"
                ))]));
            }
        };
        let clone_time_ms = elapsed_millis(clone_start);

        // Phase 1b: Register a corpus root for the clone.
        let root_id = ministr_core::ingestion::compute_root_id(&clone_result.clone_dir);
        let clone_root = ministr_core::types::CorpusRoot {
            id: root_id.clone(),
            path: clone_result.clone_dir.to_string_lossy().to_string(),
            kind: ministr_core::types::RootKind::Git,
            display_name: Some(repo_display_name(&params.repo)),
            file_count: 0,
            language_stats: std::collections::HashMap::new(),
            repo_url: Some(params.repo.clone()),
            branch: clone_result.metadata.branch.clone(),
            commit_sha: Some(clone_result.metadata.commit_sha.clone()),
            clone_timestamp: Some(clone_result.metadata.clone_timestamp.clone()),
            sparse_paths: clone_result.metadata.checked_out_paths.clone(),
        };
        if let Err(e) = storage.upsert_corpus_root(&clone_root).await {
            warn!(error = %e, repo = %params.repo, "failed to register clone corpus root");
        }

        // Phase 2: Ingest the cloned content with embeddings (root-scoped).
        let ingest_start = std::time::Instant::now();
        let ingest_result = self
            .ingestion_pipeline
            .ingest_directory_with_embeddings_rooted(
                &clone_result.clone_dir,
                storage,
                embedder,
                index,
                Some(&root_id),
                ct,
            )
            .await;
        let index_time_ms = elapsed_millis(ingest_start);

        match ingest_result {
            Ok(stats) => {
                // Update the root's file count and language stats.
                let lang_stats = compute_language_stats(&clone_result.files);
                let updated_root = ministr_core::types::CorpusRoot {
                    file_count: stats.files_indexed,
                    language_stats: lang_stats,
                    ..clone_root
                };
                if let Err(e) = storage.upsert_corpus_root(&updated_root).await {
                    warn!(error = %e, repo = %params.repo, "failed to update clone root stats");
                }

                // Record the clone in the git cache for staleness tracking.
                let git_cache_record = ministr_core::storage::GitCacheRecord {
                    repo_url: params.repo.clone(),
                    branch: params.branch.clone(),
                    commit_sha: clone_result.metadata.commit_sha.clone(),
                    clone_timestamp: clone_result.metadata.clone_timestamp.clone(),
                    clone_dir: clone_result.clone_dir.to_string_lossy().to_string(),
                    checked_out_paths: clone_result.metadata.checked_out_paths.clone(),
                };
                if let Err(e) = storage.upsert_git_cache(&git_cache_record).await {
                    warn!(error = %e, repo = %params.repo, "failed to record git cache");
                }

                // Phase 3: Re-resolve local references against newly-indexed dependency.
                let dep_graph = PackageGraph::from_cloned_repo(&clone_result.clone_dir);
                let dep_dir_str = clone_result.clone_dir.to_string_lossy().to_string();
                let dependency_refs_linked = if dep_graph.is_empty() {
                    0
                } else {
                    // `service` must be present for refresh — the tool body
                    // gates this at the top — but the check is defensive in
                    // case future callers reach this branch in daemon mode.
                    let Some(ref service) = self.service else {
                        return Err(McpError::internal_error(
                            "ministr_refresh requires local engine".to_string(),
                            None,
                        ));
                    };
                    let corpus_roots: Vec<std::path::PathBuf> = service
                        .list_corpus_roots()
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|r| r.kind == ministr_core::types::RootKind::Local)
                        .map(|r| std::path::PathBuf::from(r.path))
                        .collect();
                    match self
                        .ingestion_pipeline
                        .re_resolve_dependency_refs(
                            &dep_graph,
                            &[dep_dir_str],
                            &corpus_roots,
                            storage,
                        )
                        .await
                    {
                        Ok(count) => count,
                        Err(e) => {
                            warn!(
                                error = %e,
                                repo = %params.repo,
                                "dependency reference re-resolution failed"
                            );
                            0
                        }
                    }
                };

                debug!(
                    repo = %params.repo,
                    files_discovered = clone_result.files.len(),
                    files_indexed = stats.files_indexed,
                    sections = stats.total_sections,
                    dependency_refs_linked,
                    clone_ms = clone_time_ms,
                    index_ms = index_time_ms,
                    from_cache = clone_result.from_cache,
                    "ministr_clone success"
                );

                let mut reg = self.registry.lock().await;
                let usage_status = self.ensure_session_mut(&mut reg).budget.usage_status();
                drop(reg);

                let response = self
                    .build_response(
                        CloneResponse {
                            files_discovered: clone_result.files.len(),
                            files_indexed: stats.files_indexed,
                            sections_extracted: stats.total_sections,
                            clone_time_ms,
                            index_time_ms,
                            from_cache: clone_result.from_cache,
                            dependency_refs_linked,
                        },
                        usage_status,
                    )
                    .await;
                structured_result(&response)
            }
            Err(e) => {
                warn!(error = %e, repo = %params.repo, "ministr_clone: ingestion failed");
                Ok(CallToolResult::error(vec![Content::text(format!(
                    "clone succeeded but ingestion failed: {e}"
                ))]))
            }
        }
    }

    /// Execute the refresh pipeline for both web and git sources concurrently.
    ///
    /// Separated from the tool handler to satisfy the `too_many_lines` lint.
    pub(super) async fn refresh_all_sources(
        &self,
        params: &RefreshParams,
        storage: &Arc<SqliteStorage>,
        embedder: &dyn Embedder,
        index: &dyn VectorIndex,
    ) -> Result<CallToolResult, McpError> {
        // Run web and git refresh concurrently.
        let (web_result, git_result) = tokio::join!(
            self.refresh_web_sources(params, storage, embedder, index),
            self.refresh_git_sources(params.url.as_deref(), storage, embedder, index),
        );

        // Surface fatal web errors as tool errors.
        let (urls_checked, urls_refreshed, urls_unchanged, urls_failed, web_details) =
            match web_result {
                Ok(tuple) => tuple,
                Err(msg) => {
                    return Ok(CallToolResult::error(vec![Content::text(msg)]));
                }
            };
        let (git_checked, git_refreshed, git_unchanged, git_failed, git_details) = git_result;

        debug!(
            urls_checked,
            urls_refreshed,
            urls_unchanged,
            urls_failed,
            git_checked,
            git_refreshed,
            git_unchanged,
            git_failed,
            "ministr_refresh success"
        );

        let mut reg = self.registry.lock().await;
        let usage_status = self.ensure_session_mut(&mut reg).budget.usage_status();
        drop(reg);

        let response = self
            .build_response(
                RefreshResponse {
                    urls_checked,
                    urls_refreshed,
                    urls_unchanged,
                    urls_failed,
                    details: web_details,
                    git_repos_checked: git_checked,
                    git_repos_refreshed: git_refreshed,
                    git_repos_unchanged: git_unchanged,
                    git_repos_failed: git_failed,
                    git_details,
                },
                usage_status,
            )
            .await;
        structured_result(&response)
    }

    /// Refresh cached web URLs, returning aggregate counts and per-URL details.
    ///
    /// Returns `Err(message)` when the web refresh fails fatally (no URL filter),
    /// so the caller can surface the error as a `CallToolResult::error`.
    async fn refresh_web_sources(
        &self,
        params: &RefreshParams,
        storage: &Arc<SqliteStorage>,
        embedder: &dyn Embedder,
        index: &dyn VectorIndex,
    ) -> Result<(usize, usize, usize, usize, Vec<RefreshUrlDetailResponse>), String> {
        let Some(ref web_fetcher) = self.web_fetcher else {
            return Ok((0, 0, 0, 0, Vec::new()));
        };

        match web_fetcher
            .refresh_all(
                params.url.as_deref(),
                &self.ingestion_pipeline,
                storage.as_ref(),
                embedder,
                index,
            )
            .await
        {
            Ok(result) => {
                let details: Vec<RefreshUrlDetailResponse> = result
                    .details
                    .iter()
                    .map(|d| RefreshUrlDetailResponse {
                        url: d.url.clone(),
                        status: d.status.to_string(),
                    })
                    .collect();
                Ok((
                    result.urls_checked,
                    result.urls_refreshed,
                    result.urls_unchanged,
                    result.urls_failed,
                    details,
                ))
            }
            Err(e) => {
                if params.url.is_some() {
                    debug!(error = %e, "web refresh skipped (URL may be git)");
                    Ok((0, 0, 0, 0, Vec::new()))
                } else {
                    warn!(error = %e, "ministr_refresh web failed");
                    Err(format!("refresh failed: {e}"))
                }
            }
        }
    }

    /// Refresh all cached git clones, or a single repo if `url_filter` matches.
    ///
    /// Phase 1: check staleness of all repos concurrently (bounded by
    /// `GitFetcherConfig::refresh_concurrency`).
    /// Phase 2: re-ingest stale repos sequentially (disk-bound, not worth parallelising).
    ///
    /// Returns `(checked, refreshed, unchanged, failed, details)`.
    #[allow(clippy::too_many_lines)]
    async fn refresh_git_sources(
        &self,
        url_filter: Option<&str>,
        storage: &Arc<SqliteStorage>,
        embedder: &dyn Embedder,
        index: &dyn VectorIndex,
    ) -> (usize, usize, usize, usize, Vec<RefreshGitDetailResponse>) {
        let Some(ref git_fetcher) = self.git_fetcher else {
            return (0, 0, 0, 0, Vec::new());
        };

        let records = if let Some(url) = url_filter {
            match storage.get_git_cache(url).await {
                Ok(Some(record)) => vec![record],
                Ok(None) => return (0, 0, 0, 0, Vec::new()),
                Err(e) => {
                    warn!(error = %e, "failed to query git cache");
                    return (0, 0, 0, 0, Vec::new());
                }
            }
        } else {
            match storage.list_git_cache().await {
                Ok(records) => records,
                Err(e) => {
                    warn!(error = %e, "failed to list git cache");
                    return (0, 0, 0, 0, Vec::new());
                }
            }
        };

        // Phase 1: concurrent staleness checks + re-clones for stale repos.
        let concurrency = git_fetcher.config().refresh_concurrency;
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let mut handles = Vec::with_capacity(records.len());

        for record in records {
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("locally-owned semaphore is never closed");
            let fetcher = Arc::clone(git_fetcher);
            let paths_opt: Option<Vec<String>> = if record.checked_out_paths.is_empty() {
                None
            } else {
                Some(record.checked_out_paths.clone())
            };

            handles.push(tokio::spawn(async move {
                let result = fetcher
                    .refresh(
                        &record.repo_url,
                        paths_opt.as_deref(),
                        record.branch.as_deref(),
                        &record.commit_sha,
                    )
                    .await;
                drop(permit);
                (record, paths_opt, result)
            }));
        }

        // Phase 2: gather results and re-ingest stale repos.
        let mut checked = 0usize;
        let mut refreshed = 0usize;
        let mut unchanged = 0usize;
        let mut failed = 0usize;
        let mut details = Vec::new();

        for handle in handles {
            checked += 1;
            let Ok((record, paths_opt, refresh_result)) = handle.await else {
                failed += 1;
                warn!("git refresh task panicked");
                details.push(RefreshGitDetailResponse {
                    repo_url: String::from("<unknown>"),
                    status: "failed: task panicked".to_string(),
                });
                continue;
            };

            match refresh_result {
                Ok(None) => {
                    unchanged += 1;
                    details.push(RefreshGitDetailResponse {
                        repo_url: record.repo_url.clone(),
                        status: "unchanged".to_string(),
                    });
                }
                Ok(Some(clone_result)) => {
                    let params = CloneParams {
                        repo: record.repo_url.clone(),
                        paths: paths_opt,
                        branch: record.branch.clone(),
                    };
                    match self
                        .clone_and_ingest(
                            &params,
                            git_fetcher,
                            embedder,
                            index,
                            storage.as_ref(),
                            None,
                        )
                        .await
                    {
                        Ok(_) => {
                            refreshed += 1;
                            details.push(RefreshGitDetailResponse {
                                repo_url: record.repo_url.clone(),
                                status: format!(
                                    "updated: {} -> {}",
                                    &record.commit_sha[..7.min(record.commit_sha.len())],
                                    &clone_result.metadata.commit_sha
                                        [..7.min(clone_result.metadata.commit_sha.len())]
                                ),
                            });
                        }
                        Err(e) => {
                            failed += 1;
                            warn!(error = ?e, repo = %record.repo_url, "git refresh re-ingest failed");
                            details.push(RefreshGitDetailResponse {
                                repo_url: record.repo_url.clone(),
                                status: "failed: re-ingest error".to_string(),
                            });
                        }
                    }
                }
                Err(e) => {
                    failed += 1;
                    warn!(error = %e, repo = %record.repo_url, "git staleness check failed");
                    details.push(RefreshGitDetailResponse {
                        repo_url: record.repo_url.clone(),
                        status: format!("failed: {e}"),
                    });
                }
            }
        }

        (checked, refreshed, unchanged, failed, details)
    }
}
