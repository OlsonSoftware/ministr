//! HTTP daemon on the platform-native IPC transport.
//!
//! Exposes the ministr daemon API via axum over a Unix domain socket on
//! macOS/Linux and a named pipe on Windows. All handlers delegate to
//! [`QueryService`] via the [`CorpusRegistry`].

use std::convert::Infallible;
use std::sync::Arc;

use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_core::Stream;
use ministr_api::IpcAddr;
use tracing::info;

use crate::registry::RegistryError;
use crate::transport::Listener;

use ministr_api::ApiError;
use ministr_api::TenantId;
use ministr_api::activity::ActivityResponse;
use ministr_api::audit::AuditEntry;
use ministr_api::coherence::CoherenceEventsResponse;
use ministr_api::corpus::{
    CloneRepoRequest, CloneRepoResponse, ListCorporaResponse, RegisterCorpusRequest,
    RegisterCorpusResponse, UpdateCorpusPathsRequest,
};
use ministr_api::query;
use ministr_api::session::{CreateSessionRequest, CreateSessionResponse};
use ministr_api::status::DaemonStatus;
use ministr_core::session::AccessMode;
use ministr_core::storage::{Storage as _, SymbolFilter};
use ministr_core::types::RelationType;
use sha2::{Digest, Sha256};

use crate::activity::{ActivitySummary, record as record_activity};
use crate::convert;
use crate::state::{ACTIVITY_BUFFER_CAPACITY, AppState, COHERENCE_BUFFER_CAPACITY};

/// Read-only daemon routes — query handlers + status + ingestion-progress SSE.
///
/// Safe to gate with a read-only OAuth scope (`ministr:read`) when mounted
/// behind public auth.
pub fn corpora_read_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/corpora", get(list_corpora))
        .route("/api/v1/corpora/{id}", get(corpus_status))
        .route("/api/v1/corpora/{id}/survey", post(survey))
        .route("/api/v1/corpora/{id}/symbols", post(symbols))
        .route("/api/v1/corpora/{id}/definition/{sym}", get(definition))
        .route("/api/v1/corpora/{id}/references/{sym}", get(references))
        .route("/api/v1/corpora/{id}/impact/{sym}", get(impact))
        .route("/api/v1/corpora/{id}/dead", post(dead_code))
        .route("/api/v1/corpora/{id}/solid", post(solid))
        .route("/api/v1/corpora/{id}/read/{section}", get(read_section))
        .route("/api/v1/corpora/{id}/extract", post(extract))
        .route("/api/v1/corpora/{id}/toc", post(toc))
        .route("/api/v1/corpora/{id}/related", post(related))
        .route("/api/v1/corpora/{id}/bridge", post(bridge))
        .route("/api/v1/corpora/{id}/bridge/graph", get(bridge_graph))
        .route("/api/v1/corpora/{id}/compress", post(compress_content))
        .route("/api/v1/corpora/{id}/progress", get(ingestion_progress))
        .route("/api/v1/corpora/{id}/coherence", get(coherence_stream))
        .route("/api/v1/corpora/{id}/prefetch", get(prefetch_metrics))
        .route(
            "/api/v1/corpora/{id}/sessions/{sid}/usage",
            get(session_usage),
        )
        .route(
            "/api/v1/corpora/{id}/sessions/{sid}/read/{section}",
            get(session_read_section),
        )
        .route("/api/v1/status", get(daemon_status))
        .with_state(state)
}

/// State-mutating daemon routes — corpus + session lifecycle.
///
/// Gate behind `ministr:write` on public deployments.
pub fn corpora_write_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/corpora", post(register_corpus))
        .route("/api/v1/corpora/{id}/clone", post(clone_repo))
        .route("/api/v1/corpora/{id}", axum::routing::delete(unregister_corpus))
        .route("/api/v1/corpora/{id}/paths", put(update_corpus_paths))
        .route(
            "/api/v1/corpora/{id}/sessions",
            post(create_session).delete(clear_sessions),
        )
        .route(
            "/api/v1/corpora/{id}/sessions/{sid}",
            axum::routing::delete(destroy_session),
        )
        .route(
            "/api/v1/corpora/{id}/sessions/{sid}/dropped",
            post(drop_content),
        )
        .with_state(state)
}

/// Bundle import / export — large payloads, file IO, sensitive.
///
/// Gate behind `ministr:bundle:write` on public deployments.
pub fn corpora_bundle_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/corpora/import", post(import_bundle))
        .route("/api/v1/corpora/{id}/export", post(export_bundle))
        .with_state(state)
}

/// Claude-CLI inference handler — depends on a `claude` binary in PATH.
///
/// Mounted on local-UDS deployments via [`router`]; cloud deployments
/// (which ship without the `claude` CLI) intentionally skip this and let
/// callers receive a 404 if they discover the path.
pub fn corpora_ask_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/corpora/{id}/ask", post(ask_handler))
        .with_state(state)
}

/// Observability snapshots — recent tool-call activity + file-coherence events.
///
/// Gate behind `ministr:write` on public deployments (these leak corpus paths
/// and tool-call patterns).
pub fn observability_router(state: AppState) -> Router {
    Router::new()
        .route("/activity", get(recent_activity))
        .route("/coherence-events", get(recent_coherence_events))
        .with_state(state)
}

/// Build the daemon API router.
///
/// Composes every sub-router (read, write, bundle, ask, observability) and
/// wraps the merged tree with the `record_activity` middleware. Used by the
/// local UDS daemon; cloud deployments mount the sub-routers individually
/// with per-scope OAuth guards.
pub fn router(state: AppState) -> Router {
    corpora_read_router(state.clone())
        .merge(corpora_write_router(state.clone()))
        .merge(corpora_bundle_router(state.clone()))
        .merge(corpora_ask_router(state.clone()))
        .merge(observability_router(state.clone()))
        .layer(middleware::from_fn_with_state(state, record_activity))
}

/// `GET /activity?limit=50&since=<unix_ms>` — returns a snapshot of
/// recent tool-call activity events. Newest first. Caps at the ring
/// buffer's capacity (default 500).
#[derive(Debug, Default, serde::Deserialize)]
struct ActivityQuery {
    limit: Option<usize>,
    since: Option<u64>,
}

async fn recent_activity(
    State(state): State<AppState>,
    Query(q): Query<ActivityQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(ACTIVITY_BUFFER_CAPACITY);
    let events = if let Some(since) = q.since {
        state.activity_since(since, limit).await
    } else {
        state.recent_activity(limit).await
    };
    Json(ActivityResponse {
        events,
        buffer_capacity: ACTIVITY_BUFFER_CAPACITY,
    })
}

/// `GET /coherence-events?limit=50&since=<unix_ms>` — snapshot of recent
/// file-change events across all registered corpora. Same polling
/// contract as `/activity`.
async fn recent_coherence_events(
    State(state): State<AppState>,
    Query(q): Query<ActivityQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(COHERENCE_BUFFER_CAPACITY);
    let events = if let Some(since) = q.since {
        state.coherence_since(since, limit).await
    } else {
        state.recent_coherence(limit).await
    };
    Json(CoherenceEventsResponse {
        events,
        buffer_capacity: COHERENCE_BUFFER_CAPACITY,
    })
}

/// Start the daemon listener on the platform-native IPC endpoint.
///
/// Writes a PID file for process liveness detection and clears stale
/// endpoint artifacts from a crashed predecessor (Unix only — Windows
/// named pipes are reference-counted by the kernel and vanish with the
/// owning process). On graceful shutdown, cleans up both the endpoint
/// and PID file.
///
/// # Errors
///
/// Returns an error if the endpoint cannot be bound or another daemon
/// is running.
pub async fn start(state: AppState) -> Result<(), Box<dyn std::error::Error>> {
    let addr = ministr_api::daemon_ipc_addr();
    let data_dir = ministr_api::daemon_data_dir();
    let pid_path = ministr_api::daemon_pid_path();

    std::fs::create_dir_all(&data_dir)?;

    // Startup resilience: on Unix, a leftover socket file from a crashed
    // predecessor would make bind() fail — probe liveness and remove.
    // On Windows, named pipes don't leave stale artifacts (they're
    // refcounted kernel objects), and `first_pipe_instance(true)` in
    // `Listener::bind` turns a conflicting owner into a clear error.
    #[cfg(unix)]
    if let IpcAddr::Unix(path) = &addr
        && path.exists()
    {
        if is_daemon_alive(&addr, &pid_path).await {
            return Err("another ministr daemon is already running".into());
        }
        tracing::warn!("removing stale socket from crashed daemon");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(&pid_path);
    }

    let listener = Listener::bind(&addr)?;

    // Write PID file for liveness detection by proxies and future launches.
    let pid = std::process::id();
    std::fs::write(&pid_path, pid.to_string())?;
    info!(endpoint = %addr, pid, "daemon listening");

    // Graceful shutdown on ctrl-c or SIGTERM.
    let shutdown = shutdown_signal();
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await?;

    info!("daemon shutting down gracefully");
    cleanup_endpoint(&addr);
    let _ = std::fs::remove_file(&pid_path);
    Ok(())
}

/// Start the daemon on a caller-provided listener (for testing).
///
/// Does not write PID files or handle signals — the caller manages
/// the listener lifecycle.
///
/// # Errors
///
/// Returns an error if the axum server fails.
pub async fn serve(state: AppState, listener: Listener) -> Result<(), Box<dyn std::error::Error>> {
    axum::serve(listener, router(state)).await?;
    Ok(())
}

/// Remove any persistent artifact left behind by the endpoint.
///
/// Unix: delete the socket file. Windows: named pipes are torn down
/// automatically when the last handle is closed, so this is a no-op.
fn cleanup_endpoint(addr: &IpcAddr) {
    if let IpcAddr::Unix(path) = addr {
        let _ = std::fs::remove_file(path);
    }
}

/// Check whether a live daemon is listening at the endpoint.
///
/// Used during startup to distinguish "socket file from crashed process"
/// from "another daemon is running". On Unix we require a PID file so
/// we don't mistake a dangling client-side socket for a real daemon.
/// On Windows this path isn't exercised by [`start`] because
/// `first_pipe_instance(true)` already handles the conflict.
#[cfg(unix)]
async fn is_daemon_alive(addr: &IpcAddr, pid_path: &std::path::Path) -> bool {
    if !pid_path.exists() {
        return false;
    }
    ministr_api::transport::connect(addr).await.is_ok()
}

/// Wait for ctrl-c or SIGTERM (Unix) to initiate graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => info!("received ctrl-c, shutting down"),
            _ = sigterm.recv() => info!("received SIGTERM, shutting down"),
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
        info!("received ctrl-c, shutting down");
    }
}

// ---------------------------------------------------------------------------
// Error helper
// ---------------------------------------------------------------------------

fn err(status: StatusCode, code: &str, msg: impl std::fmt::Display) -> impl IntoResponse {
    (
        status,
        Json(ApiError {
            code: code.to_string(),
            message: msg.to_string(),
        }),
    )
}

/// Resolve a corpus ID to its `Arc<CorpusHandle>`, returning a 404
/// response on failure. The handle is detached from the registry map —
/// no `RwLockReadGuard` is held, so the handler's `.await`s never
/// serialise register / unregister.
macro_rules! get_corpus {
    ($state:expr, $id:expr) => {
        match $state.registry.get($id).await {
            Ok(handle) => handle,
            Err(e) => return err(StatusCode::NOT_FOUND, "not_found", e).into_response(),
        }
    };
}

/// Advance the session's turn counter by one and record the response's
/// token cost against the session's budget, then persist. Fire-and-forget:
/// unknown sessions and storage errors are swallowed — budget and turn
/// bookkeeping are informational for the UI and must never fail a tool
/// call.
///
/// Called by each tool handler when a `session_id` is present so the
/// observatory's turn stream and budget gauges move on every agent
/// interaction (not just session-aware reads). Pass `response_tokens`
/// measured from the serialized response body so the budget models what
/// actually landed in the agent's context.
async fn tick_session_turn(
    state: &AppState,
    corpus_id: &str,
    session_id: &str,
    tool: &str,
    response_tokens: usize,
) {
    let Ok(handle) = state.registry.get(corpus_id).await else {
        return;
    };
    let content_id = format!(
        "tool:{tool}:{ns}",
        ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    {
        let mut sessions = handle.sessions.lock().await;
        // `get_or_create` so the counter keeps ticking even after a daemon
        // restart where the `SessionRegistry` is empty — proxies memoize
        // session IDs across restarts and we'd otherwise silently no-op.
        let entry = sessions.get_or_create(session_id, None, AccessMode::ReadWrite);
        entry.session.tick();
        let _ = entry.budget.record_tokens(&content_id, response_tokens);
    }
    // Persist in a separate pass so the mutating lock is released first.
    let sessions = handle.sessions.lock().await;
    if let Some(entry) = sessions.get_session(session_id) {
        let _ = handle.storage.save_session(&entry.session).await;
    }
}

/// Count tokens in the serialized JSON form of a tool response. Used to
/// feed the session's budget — approximate but consistent across tools.
fn response_tokens<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_string(value).map_or(0, |s| ministr_core::token::count_tokens(&s))
}

/// Attach an [`ActivitySummary`] extension to a response so the activity
/// middleware records a human-readable label (the actual query / symbol
/// name / section id) instead of the empty path-derived fallback.
///
/// The middleware reads this extension after the handler returns and
/// falls back to `decode_summary(&route.path_summary)` when absent, so
/// every callable handler is safe to leave un-enriched during rollout.
fn with_summary<T: IntoResponse>(body: T, summary: String) -> axum::response::Response {
    let mut res = body.into_response();
    res.extensions_mut().insert(ActivitySummary {
        summary: Some(summary),
        ..Default::default()
    });
    res
}

/// Last `::`-delimited segment of a fully-qualified symbol id, used as
/// the human-readable name in activity summaries.
fn symbol_short_name(sym: &str) -> &str {
    sym.rsplit("::").next().unwrap_or(sym)
}

/// Source file path embedded in a `sym-…` symbol id, when present. The
/// activity dashboard groups events by file using this, so a definition
/// or references event without it is invisible in the "Code touched"
/// section.
///
/// Symbol id shape (from ministr-core):
/// `sym-<absolute_or_relative_path>::<module>::<...>::<name>`
fn file_from_symbol_id(sym: &str) -> Option<&str> {
    let stripped = sym.strip_prefix("sym-")?;
    stripped.split_once("::").map(|(file, _)| file)
}

/// Split a hierarchical section id into `(file, anchor)`. Sections look
/// like `/abs/path/foo.md#heading-slug`; the part before `#` is the file
/// and what comes after is the in-file anchor.
fn split_section_id(section: &str) -> (&str, Option<&str>) {
    match section.split_once('#') {
        Some((file, anchor)) => (file, Some(anchor)),
        None => (section, None),
    }
}

/// F3.7b — fire-and-forget audit emission helper. Emits an
/// [`AuditEntry`] when the daemon's `AppState` has a sink wired
/// (cloud mode); a no-op on self-hosted serve. `actor` comes from the
/// per-request `TenantId` extension that auth middleware populates;
/// `None` means the action was taken without authentication
/// (self-hosted serve) and the audit row's `actor` column lands NULL.
fn audit_corpus_action(
    state: &AppState,
    tenant: Option<&Extension<TenantId>>,
    action: &str,
    corpus_id: &str,
) {
    let Some(sink) = state.audit_sink.as_ref() else {
        return;
    };
    let mut entry = AuditEntry::new(action, corpus_id);
    if let Some(Extension(tid)) = tenant {
        entry = entry.with_actor(&tid.0);
    }
    sink.record(entry);
}

// ---------------------------------------------------------------------------
// Corpus management
// ---------------------------------------------------------------------------

async fn register_corpus(
    State(state): State<AppState>,
    tenant: Option<Extension<TenantId>>,
    Json(req): Json<RegisterCorpusRequest>,
) -> impl IntoResponse {
    // PHASE3 chunk 4 — when an IndexJobSink is wired (cloud mode), do
    // NOT run ingestion inline. Compute the deterministic corpus_id
    // and use the sink to either upsert-only (paths-only registration:
    // serve pod has no local source files, so dispatching an indexer
    // job would discover 0 files and pollute the queue) or upsert +
    // enqueue (when a Git/Web URL is included). The demo's "register
    // a parent corpus" pattern hits the upsert-only branch.
    if let Some(sink) = state.index_job_sink.as_ref() {
        use ministr_core::config::{CorpusSource, classify_corpus_path};
        let canonical = match ministr_core::corpus_id::canonical_corpus_paths(&req.paths) {
            Ok(c) => c,
            Err(e) => {
                return err(StatusCode::BAD_REQUEST, "register_failed", e).into_response();
            }
        };
        let corpus_id = match ministr_core::corpus_id::corpus_id_from_paths(&canonical) {
            Ok(id) => id,
            Err(e) => {
                return err(StatusCode::BAD_REQUEST, "register_failed", e).into_response();
            }
        };
        let has_remote = canonical.iter().any(|p| {
            matches!(
                classify_corpus_path(p),
                CorpusSource::Git(_) | CorpusSource::Web(_)
            )
        });
        let indexing_started = if has_remote {
            if let Err(e) = sink
                .create_pending(&corpus_id, &canonical, req.display_name.as_deref(), None)
                .await
            {
                return err(StatusCode::INTERNAL_SERVER_ERROR, "enqueue_failed", e)
                    .into_response();
            }
            true
        } else {
            if let Err(e) = sink
                .register_corpus_only(&corpus_id, &canonical, req.display_name.as_deref())
                .await
            {
                return err(StatusCode::INTERNAL_SERVER_ERROR, "register_failed", e)
                    .into_response();
            }
            false
        };
        // F3.7b — audit corpus.created on the cloud-enqueue path.
        audit_corpus_action(&state, tenant.as_ref(), "corpus.created", &corpus_id);
        return Json(RegisterCorpusResponse {
            corpus_id,
            indexing_started,
        })
        .into_response();
    }

    match state.registry.register(&req.paths).await {
        Ok((corpus_id, indexing_started)) => {
            // Apply the caller-supplied display_name override (e.g. the
            // linked-project label or `ministr_clone` repo-derived name)
            // so the tray UI shows the human-meaningful identifier rather
            // than the path-basename fallback.
            if let Some(name) = req.display_name
                && !name.is_empty()
                && let Ok(handle) = state.registry.get(&corpus_id).await
            {
                handle.info.write().await.display_name = name;
            }
            // F3.7b — audit corpus.created on the inline-register path.
            audit_corpus_action(&state, tenant.as_ref(), "corpus.created", &corpus_id);
            Json(RegisterCorpusResponse {
                corpus_id,
                indexing_started,
            })
            .into_response()
        }
        Err(e) => err(StatusCode::BAD_REQUEST, "register_failed", e).into_response(),
    }
}

/// Clone a git repo into a managed directory, register it as a new corpus,
/// and append a `[[linked]]` entry to the parent corpus's `.ministr.toml`.
///
/// The path layout: `~/.ministr/clones/{sanitized-repo}/`. Sanitisation
/// strips the protocol + replaces `/` and `:` so the path is filesystem-safe.
///
/// Steps:
///   1. Resolve target path.
///   2. `GitFetcher::clone` into target path (idempotent — re-uses cache).
///   3. Write a minimal `.ministr.toml` in the cloned tree if absent so the
///      new corpus is self-describing.
///   4. Register the new corpus with the daemon → new `corpus_id`.
///   5. Locate the parent corpus's `.ministr.toml` and append a
///      `[[linked]]` entry (idempotent — no-op if already present).
///   6. Return `CloneRepoResponse`.
#[allow(clippy::too_many_lines)] // sequential 6-step flow; splitting fragments the clone narrative
async fn clone_repo(
    State(state): State<AppState>,
    Path(parent_id): Path<String>,
    tenant: Option<Extension<TenantId>>,
    Json(req): Json<CloneRepoRequest>,
) -> impl IntoResponse {
    // PHASE3 chunk 4 — cloud-mode enqueue path. The serve pod no
    // longer clones inline; it computes a deterministic corpus_id
    // from the repo URL and enqueues a `Tenant{clone_url}` job. The
    // worker (chunk 3) does the clone + index + upload. Parent
    // lookup and parent .ministr.toml updates are skipped — the
    // linked-toml update only meaningful for self-hosted parents
    // and the worker doesn't ship a registry.
    if let Some(sink) = state.index_job_sink.as_ref() {
        // GitHub App installation-token minting is deferred — the
        // token would expire before the worker dequeues. A future
        // refinement adds `installation_id` to the Tenant trigger so
        // the worker mints at clone time. PAT-in-URL still works.
        if req.github_installation_id.is_some() {
            return err(
                StatusCode::NOT_IMPLEMENTED,
                "github_app_clone_not_supported_in_queue_mode",
                "github-app clones via installation_id not yet supported in cloud queue mode; \
                 use a PAT-in-URL clone for now".to_string(),
            )
            .into_response();
        }
        let label = req
            .label
            .clone()
            .unwrap_or_else(|| derive_repo_label(&req.repo));
        // The deterministic id is derived from the clone URL itself —
        // same URL ⇒ same corpus_id across pods, so a re-POST is
        // idempotent on the canonical id.
        let canonical = match ministr_core::corpus_id::canonical_corpus_paths(std::slice::from_ref(&req.repo)) {
            Ok(c) => c,
            Err(e) => {
                return err(StatusCode::BAD_REQUEST, "register_failed", e).into_response();
            }
        };
        let corpus_id = match ministr_core::corpus_id::corpus_id_from_paths(&canonical) {
            Ok(id) => id,
            Err(e) => {
                return err(StatusCode::BAD_REQUEST, "register_failed", e).into_response();
            }
        };
        if let Err(e) = sink
            .create_pending(
                &corpus_id,
                &canonical,
                Some(&label),
                Some(&req.repo),
            )
            .await
        {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "enqueue_failed", e)
                .into_response();
        }
        // F3.7b — audit corpus.cloned on the cloud-enqueue path.
        audit_corpus_action(&state, tenant.as_ref(), "corpus.cloned", &corpus_id);
        // Response carries placeholders for the worker-derived fields
        // (clone_dir, commit_sha, branch) — those are unknown until
        // the worker clones. The progress SSE surfaces the transitions.
        return Json(CloneRepoResponse {
            corpus_id,
            clone_dir: String::new(),
            label,
            commit_sha: String::new(),
            branch: req.branch.clone().unwrap_or_default(),
            linked_toml_updated: false,
            indexing_started: true,
        })
        .into_response();
    }

    // 1. Lookup parent corpus paths via its info handle so we can locate
    //    the parent's `.ministr.toml` later.
    let parent_paths = match state.registry.get(&parent_id).await {
        Ok(handle) => {
            let info = handle.info.read().await;
            info.paths.clone()
        }
        Err(e) => {
            return err(StatusCode::NOT_FOUND, "parent_not_found", e).into_response();
        }
    };

    // 1b. F2.1 — mint a GitHub App installation token when the caller
    //     supplies an `installation_id` AND a minter is wired. The
    //     resulting URL is identical in shape to a PAT URL
    //     (`https://x-access-token:<token>@github.com/...`), so the
    //     downstream `GitFetcher::clone` path stays untouched. The token
    //     is never persisted; on cache miss it lives only for the
    //     duration of this request.
    let effective_repo: String = match (
        req.github_installation_id.as_ref(),
        state.installation_minter.as_ref(),
    ) {
        (Some(installation_id), Some(minter)) => match minter.mint(installation_id).await {
            Ok(token) => match inject_github_app_token(&req.repo, &token) {
                Ok(url) => url,
                Err(e) => {
                    return err(StatusCode::BAD_REQUEST, "github_url_invalid", e).into_response();
                }
            },
            Err(e) => {
                return err(
                    StatusCode::BAD_GATEWAY,
                    "github_app_token_mint_failed",
                    e.to_string(),
                )
                .into_response();
            }
        },
        (Some(_), None) => {
            return err(
                StatusCode::BAD_REQUEST,
                "github_app_not_configured",
                "this deployment has no GitHub App; supply a PAT in the URL instead".to_string(),
            )
            .into_response();
        }
        _ => req.repo.clone(),
    };

    // 2. Clone via GitFetcher into ~/.ministr/clones/{sanitised}/.
    let git_fetcher = ministr_core::git::GitFetcher::with_defaults();
    let paths_ref: Option<Vec<String>> = if req.paths.is_empty() {
        None
    } else {
        Some(req.paths.clone())
    };
    let clone_result = match git_fetcher
        .clone(
            &effective_repo,
            paths_ref.as_deref(),
            req.branch.as_deref(),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return err(StatusCode::BAD_GATEWAY, "clone_failed", e).into_response();
        }
    };

    let clone_dir = clone_result.clone_dir.clone();
    let clone_dir_str = clone_dir.to_string_lossy().to_string();

    // Derive the linked-project label from the URL, or use the caller-
    // supplied one. The fallback heuristic mirrors `git_repo_display_name`
    // in ministr-cli/src/ingestion.rs.
    let label = req
        .label
        .clone()
        .unwrap_or_else(|| derive_repo_label(&req.repo));

    // 3. (no-op) Earlier revisions wrote a `.ministr.toml` with
    //    `[corpus] paths = ["."]` inside the cloned tree to make the
    //    corpus self-describing, but the daemon registers the clone path
    //    directly and `ingest_paths_with_embeddings` discovers files by
    //    walking that path — it never reads a per-corpus `.ministr.toml`
    //    for paths/ignore config. The stray file is just one more thing
    //    for the file watcher to debounce on and confused the manifest
    //    when the path was registered with a trailing `/.`. Skipped.

    // 4. Register the new corpus.
    let new_paths: Vec<String> = vec![clone_dir_str.clone()];
    let (new_corpus_id, indexing_started) = match state.registry.register(&new_paths).await {
        Ok(v) => v,
        Err(e) => {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "register_failed", e).into_response();
        }
    };

    // 4b. Override the daemon-derived display_name (which would otherwise
    //     be the basename of the content-hashed clone dir — e.g.
    //     `cbbbc0ee0e720d13` — and surfaces that in the tray UI) with the
    //     friendly label we derived from the repo URL.
    if let Ok(handle) = state.registry.get(&new_corpus_id).await {
        let mut info = handle.info.write().await;
        info.display_name.clone_from(&label);
    }

    // 5. Append `[[linked]]` to the parent's `.ministr.toml`.
    let linked_toml_updated = if let Some(parent_toml) = find_ministr_toml(&parent_paths) {
        match append_linked_entry(&parent_toml, &clone_dir_str, &label).await {
            Ok(updated) => updated,
            Err(e) => {
                tracing::warn!(
                    parent_toml = %parent_toml.display(),
                    error = %e,
                    "clone succeeded but parent .ministr.toml update failed"
                );
                false
            }
        }
    } else {
        tracing::warn!(
            parent_id = %parent_id,
            "no .ministr.toml found in parent paths; linked entry not written"
        );
        false
    };

    // F3.7b — audit corpus.cloned on the inline clone path.
    audit_corpus_action(&state, tenant.as_ref(), "corpus.cloned", &new_corpus_id);

    Json(CloneRepoResponse {
        corpus_id: new_corpus_id,
        clone_dir: clone_dir_str,
        label,
        commit_sha: clone_result.metadata.commit_sha.clone(),
        branch: clone_result.metadata.branch.clone().unwrap_or_default(),
        linked_toml_updated,
        indexing_started,
    })
    .into_response()
}

/// Splice a GitHub App installation access token into an `https://`
/// clone URL using the documented `x-access-token` username scheme
/// (per GitHub Docs — Generating an installation access token):
/// `https://x-access-token:<token>@github.com/owner/repo.git`.
///
/// Refuses non-`https://` schemes (SSH URLs cannot carry a token) and
/// URLs that already contain credentials (the caller should pick PAT
/// OR App, not both).
///
/// Returns the rewritten URL on success.
fn inject_github_app_token(repo: &str, token: &str) -> Result<String, String> {
    let Some(rest) = repo.strip_prefix("https://") else {
        return Err(
            "GitHub App installation tokens require an https:// clone URL".to_string(),
        );
    };
    if rest.contains('@') {
        return Err(
            "clone URL already contains credentials — pick PAT or App, not both"
                .to_string(),
        );
    }
    Ok(format!("https://x-access-token:{token}@{rest}"))
}

/// Sanitise a repo URL into a filesystem-safe label.
///
/// `https://github.com/owner/repo.git` → `owner-repo`.
/// `git@github.com:owner/repo.git`      → `owner-repo`.
fn derive_repo_label(repo: &str) -> String {
    let stripped = repo.trim_end_matches(".git").rsplit_once('/').map_or_else(
        || repo.to_string(),
        |(prefix, last)| {
            let owner = prefix.rsplit_once(['/', ':']).map_or("", |(_, o)| o);
            if owner.is_empty() {
                last.to_string()
            } else {
                format!("{owner}-{last}")
            }
        },
    );
    stripped
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Locate the `.ministr.toml` for a corpus by scanning its registered
/// paths. Returns the path of the first existing `.ministr.toml` found by
/// walking each registered path's ancestors up to a reasonable depth.
fn find_ministr_toml(paths: &[String]) -> Option<std::path::PathBuf> {
    for p in paths {
        let mut current: Option<&std::path::Path> = Some(std::path::Path::new(p));
        let mut depth = 0;
        while let Some(dir) = current
            && depth < 6
        {
            let candidate = dir.join(".ministr.toml");
            if candidate.exists() {
                return Some(candidate);
            }
            current = dir.parent();
            depth += 1;
        }
    }
    None
}

/// Append a `[[linked]]` entry to `toml_path` if one with the same `path`
/// isn't already present. Returns `true` when the file was modified.
///
/// Uses `toml_edit` to preserve comments and formatting.
async fn append_linked_entry(
    toml_path: &std::path::Path,
    linked_path: &str,
    label: &str,
) -> Result<bool, std::io::Error> {
    let raw = tokio::fs::read_to_string(toml_path).await?;
    let mut doc: toml_edit::DocumentMut = raw.parse().map_err(|e: toml_edit::TomlError| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    })?;

    // Check existing [[linked]] entries — skip if the same path is already
    // linked. Matching on `path` (rather than `label`) prevents accidental
    // double-links when an agent calls clone twice on the same repo.
    if let Some(linked) = doc
        .get("linked")
        .and_then(toml_edit::Item::as_array_of_tables)
    {
        for table in linked {
            if table
                .get("path")
                .and_then(|v| v.as_str())
                .is_some_and(|existing| existing == linked_path)
            {
                return Ok(false);
            }
        }
    }

    // Append a new [[linked]] table.
    let linked_array = doc
        .entry("linked")
        .or_insert_with(|| toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
    if let Some(arr) = linked_array.as_array_of_tables_mut() {
        let mut new_table = toml_edit::Table::new();
        new_table["path"] = toml_edit::value(linked_path);
        new_table["label"] = toml_edit::value(label);
        arr.push(new_table);
    }

    tokio::fs::write(toml_path, doc.to_string()).await?;
    Ok(true)
}

async fn list_corpora(
    State(state): State<AppState>,
    full_tenant: Option<axum::extract::Extension<ministr_api::TenantId>>,
) -> impl IntoResponse {
    let all = state.registry.list().await;

    // F3.2-iii — when cloud mode wires a visibility filter AND the
    // auth middleware populated a TenantId, filter the list to own +
    // ACL-granted corpora. Self-hosted serve has no filter and no
    // TenantId; the list returns every in-memory corpus.
    let filtered = match (&state.corpus_visibility, full_tenant) {
        (Some(filter), Some(axum::extract::Extension(tenant_id))) => {
            match filter.visible_corpus_ids(tenant_id.as_str()).await {
                Ok(Some(allow)) => {
                    let allow_set: std::collections::HashSet<&str> =
                        allow.iter().map(String::as_str).collect();
                    all.into_iter()
                        .filter(|c| allow_set.contains(c.id.as_str()))
                        .collect()
                }
                Ok(None) => all,
                Err(e) => {
                    // Fail closed on storage errors: surface an empty
                    // list rather than leak cross-tenant rows. The
                    // tracing line above is the operator-visible
                    // signal.
                    tracing::warn!(
                        error = %e,
                        subject = %tenant_id.as_str(),
                        "corpus visibility lookup failed — failing closed with empty list",
                    );
                    Vec::new()
                }
            }
        }
        _ => all,
    };

    Json(ListCorporaResponse { corpora: filtered })
}

async fn corpus_status(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.registry.list().await.into_iter().find(|c| c.id == id) {
        Some(info) => Json(info).into_response(),
        None => err(StatusCode::NOT_FOUND, "not_found", format!("corpus '{id}'")).into_response(),
    }
}

async fn unregister_corpus(
    State(state): State<AppState>,
    Path(id): Path<String>,
    tenant: Option<Extension<TenantId>>,
) -> impl IntoResponse {
    match state.registry.unregister(&id).await {
        Ok(()) => {
            // F3.7b — audit corpus.deleted only on actual removal. A
            // re-DELETE that misses falls through to the NotFound arm
            // below and never pollutes the audit feed.
            audit_corpus_action(&state, tenant.as_ref(), "corpus.deleted", &id);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => err(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    }
}

/// `PUT /api/v1/corpora/{id}/paths` — replace the corpus's path set without
/// dropping its sessions. The new paths must canonicalise to the same id;
/// see [`CorpusRegistry::update_corpus_paths`].
async fn update_corpus_paths(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCorpusPathsRequest>,
) -> impl IntoResponse {
    match state.registry.update_corpus_paths(&id, &req.paths).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e @ RegistryError::NotFound { .. }) => {
            err(StatusCode::NOT_FOUND, "not_found", e).into_response()
        }
        Err(e @ RegistryError::IdentityChanged { .. }) => {
            err(StatusCode::BAD_REQUEST, "identity_changed", e).into_response()
        }
        Err(e) => err(StatusCode::BAD_REQUEST, "update_failed", e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Query endpoints
// ---------------------------------------------------------------------------

async fn survey(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<query::SurveyRequest>,
) -> impl IntoResponse {
    let _permit = state.query_semaphore.acquire().await;
    let handle = get_corpus!(&state, &id);
    let top_k = req.top_k.unwrap_or(10);
    let session_id = req.session_id.clone();
    let summary = format!("\"{}\" (top_k={top_k})", req.query);
    let result = handle.service.survey(&req.query, top_k).await;
    drop(handle);
    match result {
        Ok(results) => {
            let body = query::SurveyResponse {
                results: results.into_iter().map(convert::survey_result).collect(),
                deduplicated_count: None,
                usage_status: None,
            };
            if let Some(sid) = session_id {
                tick_session_turn(&state, &id, &sid, "survey", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn symbols(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<query::SymbolsRequest>,
) -> impl IntoResponse {
    let _permit = state.query_semaphore.acquire().await;
    let handle = get_corpus!(&state, &id);
    let limit = req.limit.unwrap_or(20);
    let session_id = req.session_id.clone();
    let summary = {
        let mut parts = vec![format!("\"{}\"", req.query)];
        if let Some(k) = req.kind.as_ref() {
            parts.push(format!("kind={k}"));
        }
        if let Some(m) = req.module.as_ref() {
            parts.push(format!("module={m}"));
        }
        if let Some(v) = req.visibility.as_ref() {
            parts.push(format!("visibility={v}"));
        }
        parts.join(" · ")
    };
    let filter = SymbolFilter {
        name: Some(req.query),
        name_exact: None,
        kind: req.kind,
        visibility: req.visibility,
        module: req.module,
        file_path: None,
    };
    let result = handle.service.search_symbols(&filter).await;
    drop(handle);
    match result {
        Ok(records) => {
            let body = query::SymbolsResponse {
                symbols: records
                    .into_iter()
                    .take(limit)
                    .map(convert::symbol_from_record)
                    .collect(),
            };
            let summary = format!("{summary} ({n})", n = body.symbols.len());
            if let Some(sid) = session_id {
                tick_session_turn(&state, &id, &sid, "symbols", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

/// Optional `?session_id=X` query for the session-less GET routes
/// (`definition`, `references`). Lets the proxy tick the session's turn
/// counter without converting these routes to session-scoped variants.
#[derive(Debug, Default, serde::Deserialize)]
struct SessionQuery {
    session_id: Option<String>,
}

async fn definition(
    State(state): State<AppState>,
    Path((id, sym)): Path<(String, String)>,
    Query(q): Query<SessionQuery>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let result = handle.service.get_symbol_definition(&sym).await;
    drop(handle);
    match result {
        Ok(def) => {
            let body = convert::symbol_definition(def);
            let summary = match file_from_symbol_id(&sym) {
                Some(file) => format!("{name} — {file}", name = symbol_short_name(&sym)),
                None => symbol_short_name(&sym).to_string(),
            };
            if let Some(sid) = q.session_id {
                tick_session_turn(&state, &id, &sid, "definition", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    }
}

async fn references(
    State(state): State<AppState>,
    Path((id, sym)): Path<(String, String)>,
    Query(q): Query<SessionQuery>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let result = handle.service.get_symbol_references(&sym, None).await;
    drop(handle);
    match result {
        Ok(refs) => {
            let body = query::ReferencesResponse {
                references: refs.into_iter().map(convert::symbol_reference).collect(),
            };
            let n = body.references.len();
            let summary = match file_from_symbol_id(&sym) {
                Some(file) => format!("{name} — {file} ({n})", name = symbol_short_name(&sym)),
                None => format!("{name} ({n})", name = symbol_short_name(&sym)),
            };
            if let Some(sid) = q.session_id {
                tick_session_turn(&state, &id, &sid, "references", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct ImpactQuery {
    #[serde(default)]
    max_depth: Option<u32>,
    #[serde(default)]
    session_id: Option<String>,
}

async fn impact(
    State(state): State<AppState>,
    Path((id, sym)): Path<(String, String)>,
    Query(q): Query<ImpactQuery>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let max_depth = q.max_depth.unwrap_or(3);
    let result = handle.service.compute_impact(&sym, max_depth).await;
    drop(handle);
    match result {
        Ok(r) => {
            let body = convert::impact_response(r);
            let summary = format!(
                "{name} — {symbols} callers, {files} files, {risk:?} risk",
                name = symbol_short_name(&sym),
                symbols = body.symbols,
                files = body.files,
                risk = body.risk,
            );
            if let Some(sid) = q.session_id {
                tick_session_turn(&state, &id, &sid, "impact", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn dead_code(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<SessionQuery>,
    Json(req): Json<query::DeadCodeRequest>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let min_lines = req.min_lines.unwrap_or(1);
    let limit = req.limit.unwrap_or(50);
    let result = handle
        .service
        .find_dead_code(req.kind.as_deref(), req.module.as_deref(), min_lines, limit)
        .await;
    drop(handle);
    match result {
        Ok(syms) => {
            let symbols: Vec<query::DeadSymbol> =
                syms.into_iter().map(convert::dead_symbol).collect();
            let total = symbols.len();
            let body = query::DeadCodeResponse { symbols, total };
            let summary = format!("{total} dead-code candidates");
            if let Some(sid) = q.session_id {
                tick_session_turn(&state, &id, &sid, "dead", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn solid(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<SessionQuery>,
    Json(req): Json<query::SolidRequest>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let params = convert::api_solid_request_to_service(req);
    let result = handle.service.detect_solid_violations(&params).await;
    drop(handle);
    match result {
        Ok(findings) => {
            let api_findings: Vec<query::SolidFinding> =
                findings.into_iter().map(convert::solid_finding).collect();
            let total = api_findings.len();
            let body = query::SolidResponse {
                findings: api_findings,
                total,
            };
            let summary = format!("{total} SOLID findings");
            if let Some(sid) = q.session_id {
                tick_session_turn(&state, &id, &sid, "solid", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn read_section(
    State(state): State<AppState>,
    Path((id, section)): Path<(String, String)>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);

    // Check prefetch cache for a warm hit.
    let warm_detail = {
        let mut prefetch = handle.prefetch.lock().await;
        prefetch
            .try_serve(&section)
            .map(|entry| ministr_core::service::SectionDetail {
                section_id: entry.content_id.clone(),
                heading_path: entry.heading_path.clone().unwrap_or_default(),
                text: entry.text.clone(),
                summary: entry.summary.clone(),
                claims_available: entry.claims_available,
            })
    };

    let read_result = if let Some(detail) = warm_detail {
        tracing::debug!(section_id = %section, "daemon read: warm cache hit");
        Ok(detail)
    } else {
        handle.service.read_section(&section).await
    };

    match read_result {
        Ok(detail) => {
            // Clone Arcs before dropping the registry guard.
            let storage = Arc::clone(&handle.storage);
            let prefetch = Arc::clone(&handle.prefetch);
            let index = Arc::clone(&handle.index);
            let embedder = Arc::clone(state.registry.embedder());
            let section_clone = section.clone();
            drop(handle);

            // Spawn background prefetch (don't block the response).
            tokio::spawn(async move {
                trigger_prefetch(
                    &section_clone,
                    &storage,
                    &prefetch,
                    embedder.as_ref(),
                    index.as_ref(),
                )
                .await;
            });
            Json(convert::section_detail(detail)).into_response()
        }
        Err(e) => err(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    }
}

/// Session-aware read: records delivery in the session shadow + budget tracker.
///
/// Used by the MCP proxy so that `ministr_usage` reflects actual token usage.
async fn session_read_section(
    State(state): State<AppState>,
    Path((id, sid, section)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);

    // Check prefetch cache for a warm hit.
    let warm_detail = {
        let mut prefetch = handle.prefetch.lock().await;
        prefetch
            .try_serve(&section)
            .map(|entry| ministr_core::service::SectionDetail {
                section_id: entry.content_id.clone(),
                heading_path: entry.heading_path.clone().unwrap_or_default(),
                text: entry.text.clone(),
                summary: entry.summary.clone(),
                claims_available: entry.claims_available,
            })
    };

    let read_result = if let Some(detail) = warm_detail {
        tracing::debug!(section_id = %section, "daemon session_read: warm cache hit");
        Ok(detail)
    } else {
        handle.service.read_section(&section).await
    };

    match read_result {
        Ok(detail) => {
            // Record delivery in the session shadow + budget tracker.
            {
                let token_count = ministr_core::token::count_tokens(&detail.text);
                let content_id = ministr_core::types::ContentId(section.clone());
                let content_hash = {
                    let mut hasher = Sha256::new();
                    hasher.update(detail.text.as_bytes());
                    format!("{:x}", hasher.finalize())
                };
                let mut sessions = handle.sessions.lock().await;
                // Get or create the session — the proxy may hold a stale
                // session ID from before a daemon restart.
                let entry = sessions.get_or_create(&sid, None, AccessMode::ReadWrite);
                let turn = entry.session.current_turn() + 1;
                // A re-read of content that fell out of the window is a
                // fault signal (the agent "forgot" it); a fresh read or a
                // still-in-window re-read is `Good`.
                let rating = if entry.session.is_delivered(&content_id)
                    && !entry.budget.is_in_window(&section)
                {
                    ministr_core::session::memory::AccessRating::Again
                } else {
                    ministr_core::session::memory::AccessRating::Good
                };
                entry.session.record_delivery(
                    &content_id,
                    ministr_core::types::Resolution::Section,
                    token_count,
                    turn,
                    content_hash,
                );
                // Populate the FSRS memory tracker so retrievability scores
                // exist for eviction decisions under `DropPolicy::Fsrs`.
                entry.memory.record_access(&section, turn, rating);
                // Use the memory-aware variant so FSRS actually consults
                // retrievability. FIFO/LRU ignore the scores, so this call
                // is safe for all policies.
                let _ = entry.budget.record_tokens_with_memory(
                    &section,
                    token_count,
                    &entry.memory,
                    turn,
                );
            }

            // Persist session to SQLite so budget survives daemon restarts
            // and the tray app can show accurate token usage.
            {
                let sessions = handle.sessions.lock().await;
                if let Some(entry) = sessions.get_session(&sid) {
                    let _ = handle.storage.save_session(&entry.session).await;
                }
            }

            // Clone Arcs before dropping the registry guard.
            let storage = Arc::clone(&handle.storage);
            let prefetch = Arc::clone(&handle.prefetch);
            let index = Arc::clone(&handle.index);
            let embedder = Arc::clone(state.registry.embedder());
            let section_clone = section.clone();
            drop(handle);

            // Spawn background prefetch (don't block the response).
            tokio::spawn(async move {
                trigger_prefetch(
                    &section_clone,
                    &storage,
                    &prefetch,
                    embedder.as_ref(),
                    index.as_ref(),
                )
                .await;
            });
            Json(convert::section_detail(detail)).into_response()
        }
        Err(e) => err(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    }
}

async fn extract(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<query::ExtractRequest>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let session_id = req.session_id.clone();
    let summary = {
        let (file, anchor) = split_section_id(&req.section_id);
        let head = match anchor {
            Some(a) => format!("{a} — {file}"),
            None => file.to_string(),
        };
        match req.query.as_deref() {
            Some(q) => format!("{head} · \"{q}\""),
            None => head,
        }
    };
    let result = handle
        .service
        .extract_claims(&req.section_id, req.query.as_deref())
        .await;
    drop(handle);
    match result {
        Ok(claims) => {
            let body = query::ExtractResponse {
                claims: claims.into_iter().map(convert::claim_result).collect(),
            };
            let summary = format!("{summary} ({n})", n = body.claims.len());
            if let Some(sid) = session_id {
                tick_session_turn(&state, &id, &sid, "extract", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn toc(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<query::TocRequest>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let offset = req.offset.unwrap_or(0);
    let limit = req.limit.unwrap_or(100);
    let session_id = req.session_id.clone();
    let summary = req.document_id.as_deref().unwrap_or("<root>").to_string();
    let result = handle.service.toc(req.document_id.as_deref()).await;
    drop(handle);
    match result {
        Ok(entries) => {
            let total = entries.len();
            let body = query::TocResponse {
                entries: entries
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .map(convert::toc_entry)
                    .collect(),
                total,
            };
            let summary = format!("{summary} ({total})");
            if let Some(sid) = session_id {
                tick_session_turn(&state, &id, &sid, "toc", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn related(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<query::RelatedRequest>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let relation_types: Option<Vec<RelationType>> = if req.relation_types.is_empty() {
        None
    } else {
        Some(
            req.relation_types
                .iter()
                .filter_map(|s| RelationType::parse(s))
                .collect(),
        )
    };
    let session_id = req.session_id.clone();
    let summary = req.claim_id.clone();
    let result = handle
        .service
        .related_claims(&req.claim_id, relation_types.as_deref())
        .await;
    drop(handle);
    match result {
        Ok(claims) => {
            let body = query::RelatedResponse {
                claims: claims.into_iter().map(convert::related_claim).collect(),
            };
            let summary = format!("{summary} ({n})", n = body.claims.len());
            if let Some(sid) = session_id {
                tick_session_turn(&state, &id, &sid, "related", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn bridge(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<query::BridgeRequest>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let limit = req.limit.unwrap_or(50);
    let session_id = req.session_id.clone();
    let summary = {
        let mut parts: Vec<String> = Vec::new();
        if let Some(q) = req.query.as_deref() {
            parts.push(format!("\"{q}\""));
        }
        if let Some(k) = req.kind.as_deref() {
            parts.push(format!("kind={k}"));
        }
        if let Some(l) = req.source_language.as_deref() {
            parts.push(format!("lang={l}"));
        }
        if parts.is_empty() {
            "all bridges".to_string()
        } else {
            parts.join(" · ")
        }
    };
    let result = handle
        .service
        .query_bridges(
            req.query.as_deref(),
            req.kind.as_deref(),
            req.source_language.as_deref(),
            None,
        )
        .await;
    drop(handle);
    match result {
        Ok(links) => {
            let body = query::BridgeResponse {
                links: links
                    .into_iter()
                    .take(limit)
                    .map(convert::bridge_link)
                    .collect(),
            };
            let summary = format!("{summary} ({n})", n = body.links.len());
            if let Some(sid) = session_id {
                tick_session_turn(&state, &id, &sid, "bridge", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

/// F3.6-a — query filters for the bridge graph endpoint.
#[derive(serde::Deserialize)]
struct BridgeGraphQuery {
    /// Filter to links touching this file (export OR import side).
    #[serde(default)]
    file: Option<String>,
    /// Filter by bridge kind (`tauri_command`, `pyo3`, `napi`, …).
    #[serde(default)]
    kind: Option<String>,
    /// Filter by source language (export side language).
    #[serde(default)]
    language: Option<String>,
}

/// F3.6-a — `GET /api/v1/corpora/{id}/bridge/graph`.
///
/// Returns the cross-language bridge graph as `{nodes, edges}` for
/// the F3.6-b web visualizer (and any other downstream renderer).
/// Query params `file`, `kind`, `language` pass through to
/// `query_bridges` for server-side filtering.
async fn bridge_graph(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<BridgeGraphQuery>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let result = handle
        .service
        .query_bridges(
            None,
            q.kind.as_deref(),
            q.language.as_deref(),
            q.file.as_deref(),
        )
        .await;
    drop(handle);
    match result {
        Ok(links) => {
            let graph = convert::bridge_links_to_graph(&links);
            let summary = format!("{} nodes · {} edges", graph.nodes.len(), graph.edges.len());
            with_summary(Json(graph), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Compress
// ---------------------------------------------------------------------------

async fn compress_content(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ministr_api::session::CompressRequest>,
) -> impl IntoResponse {
    let _permit = state.query_semaphore.acquire().await;
    let handle = get_corpus!(&state, &id);
    let session_id = req.session_id.clone();
    let summary = format!("compress {n} items", n = req.content_ids.len());
    let result = handle.service.compress_content(&req.content_ids).await;
    drop(handle);
    match result {
        Ok(items) => {
            let body = ministr_api::session::CompressResponse {
                summaries: items.into_iter().map(convert::compressed_item).collect(),
            };
            if let Some(sid) = session_id {
                tick_session_turn(&state, &id, &sid, "compress", response_tokens(&body)).await;
            }
            with_summary(Json(body), summary)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "compress_failed", e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Ask (sub-inference)
// ---------------------------------------------------------------------------

async fn ask_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<query::AskRequest>,
) -> impl IntoResponse {
    let _permit = state.query_semaphore.acquire().await;
    let handle = get_corpus!(&state, &id);
    let session_id = req.session_id.clone();

    let result = crate::ask::ask(
        &req.query,
        &handle.service,
        &handle.storage,
        state.inference.as_ref(),
    )
    .await;
    drop(handle);
    match result {
        Ok(result) => {
            let body = query::AskResponse {
                answer: result.answer,
                source_ids: result.source_ids,
                cached: result.cached,
                model: result.model,
            };
            if let Some(sid) = session_id {
                tick_session_turn(&state, &id, &sid, "ask", response_tokens(&body)).await;
            }
            Json(body).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "ask_failed", e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Ingestion progress SSE
// ---------------------------------------------------------------------------

async fn ingestion_progress(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // PHASE3 chunk 4 — when an `IndexJobSink` is wired (cloud mode),
    // poll `latest_for_corpus` against Postgres instead of reading the
    // in-memory `IngestionProgress` (which belongs to the worker pod,
    // not the serve pod, post-split).
    if let Some(sink) = state.index_job_sink.clone() {
        let stream = queue_progress_stream(sink, id);
        return Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    let progress = {
        let corpora = state.registry.corpora().read().await;
        match corpora.get(&id) {
            Some(handle) => Arc::clone(&handle.progress),
            None => {
                return err(StatusCode::NOT_FOUND, "not_found", format!("corpus '{id}'"))
                    .into_response();
            }
        }
    };

    let stream = progress_stream(progress);
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn progress_stream(
    progress: Arc<ministr_core::ingestion::IngestionProgress>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            interval.tick().await;
            let status_code = progress.status();
            let status = match status_code {
                0 => "pending",
                1 => "running",
                _ => "complete",
            };
            let event = ministr_api::corpus::IngestionProgressEvent {
                status: status.to_string(),
                phase: progress.phase().as_str().to_string(),
                files_total: progress.files_total(),
                files_done: progress.files_done(),
                sections_done: progress.sections_done(),
                embeddings_total: progress.embeddings_total(),
                embeddings_done: progress.embeddings_done(),
                current_file: progress.current_file(),
                error: None,
            };
            if let Ok(json) = serde_json::to_string(&event) {
                yield Ok(Event::default().data(json));
            }
            // Stop streaming once ingestion is complete.
            if status_code >= 2 {
                break;
            }
        }
    }
}

/// PHASE3 chunk 4 — Postgres-backed progress stream. Polls
/// `latest_for_corpus` every 500ms, emits an [`IngestionProgressEvent`]
/// matching the existing wire shape, and closes when the latest job is
/// in a terminal state (`Completed` / `Failed`). On lookup error or
/// `None` (no job yet), emits a `pending` placeholder so the demo
/// client doesn't see a dead stream while the row is being written.
///
/// PHASE4 chunk 6: the terminal event carries `status = "complete"` or
/// `"failed"` (matching the doc'd wire shape and what `cloud_demo`
/// already checks for), plus the snapshot's `error` field on failure
/// so clients don't need a follow-up GET to surface the cause.
fn queue_progress_stream(
    sink: Arc<dyn ministr_api::IndexJobSink>,
    corpus_id: String,
) -> impl Stream<Item = Result<Event, Infallible>> {
    use ministr_api::IndexJobStatus;
    async_stream::stream! {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            interval.tick().await;
            let snapshot = match sink.latest_for_corpus(&corpus_id).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        corpus_id = %corpus_id,
                        error = %e,
                        "queue progress lookup failed — yielding pending placeholder"
                    );
                    None
                }
            };
            let (status, terminal) = match snapshot.as_ref().map(|s| s.status) {
                Some(IndexJobStatus::Pending) | None => ("pending", false),
                Some(IndexJobStatus::Running) => ("running", false),
                Some(IndexJobStatus::Completed) => ("complete", true),
                Some(IndexJobStatus::Failed) => ("failed", true),
            };
            let error = if terminal && status == "failed" {
                snapshot.as_ref().and_then(|s| s.error.clone())
            } else {
                None
            };
            // PHASE5 chunk 3 — populate embeddings_* + sections_done
            // from the snapshot now that JobProgress carries them.
            // Pre-chunk-3 these were hardcoded to 0 because the wire
            // shape clipped the embedder's per-batch updates.
            let event = ministr_api::corpus::IngestionProgressEvent {
                status: status.to_string(),
                phase: snapshot
                    .as_ref()
                    .map_or_else(String::new, |s| s.stage.clone()),
                files_total: snapshot.as_ref().map_or(0, |s| {
                    usize::try_from(s.total_files).unwrap_or(usize::MAX)
                }),
                files_done: snapshot.as_ref().map_or(0, |s| {
                    usize::try_from(s.processed_files).unwrap_or(usize::MAX)
                }),
                sections_done: snapshot.as_ref().map_or(0, |s| {
                    usize::try_from(s.sections_done).unwrap_or(usize::MAX)
                }),
                embeddings_total: snapshot.as_ref().map_or(0, |s| {
                    usize::try_from(s.embeddings_total).unwrap_or(usize::MAX)
                }),
                embeddings_done: snapshot.as_ref().map_or(0, |s| {
                    usize::try_from(s.embeddings_done).unwrap_or(usize::MAX)
                }),
                current_file: snapshot
                    .as_ref()
                    .and_then(|s| s.current_file.clone())
                    .unwrap_or_default(),
                error,
            };
            if let Ok(json) = serde_json::to_string(&event) {
                yield Ok(Event::default().data(json));
            }
            if terminal {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Coherence SSE
// ---------------------------------------------------------------------------

async fn coherence_stream(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let coherence_tx = {
        let corpora = state.registry.corpora().read().await;
        match corpora.get(&id) {
            Some(handle) => handle.coherence_tx.clone(),
            None => {
                return err(StatusCode::NOT_FOUND, "not_found", format!("corpus '{id}'"))
                    .into_response();
            }
        }
    };

    let mut rx = coherence_tx.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(json) = serde_json::to_string(&event) {
                        yield Ok::<_, Infallible>(Event::default().event("coherence").data(json));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            }
        }
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

// ---------------------------------------------------------------------------
// Bundle export/import
// ---------------------------------------------------------------------------

async fn export_bundle(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let (data_dir, model_name, dimension) = {
        let corpora = state.registry.corpora().read().await;
        match corpora.get(&id) {
            Some(handle) => (
                handle.data_dir.clone(),
                state.registry.config().default_model.clone(),
                state.registry.embedder().dimension(),
            ),
            None => {
                return err(StatusCode::NOT_FOUND, "not_found", format!("corpus '{id}'"))
                    .into_response();
            }
        }
    };

    let output_path = data_dir.join(format!("{id}.ministr-index"));
    let manifest = ministr_core::bundle::BundleManifest {
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

    match ministr_core::bundle::export_bundle(&data_dir, &output_path, &manifest) {
        Ok(path) => {
            // Re-read manifest from the exported bundle for accurate counts.
            let final_manifest = ministr_core::bundle::read_manifest(&path).unwrap_or(manifest);
            Json(ministr_api::corpus::ExportBundleResponse {
                bundle_path: path.display().to_string(),
                manifest: convert::bundle_manifest(&final_manifest),
            })
            .into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "export_failed", e).into_response(),
    }
}

async fn import_bundle(
    State(state): State<AppState>,
    Json(req): Json<ministr_api::corpus::ImportBundleRequest>,
) -> impl IntoResponse {
    let bundle_path = std::path::PathBuf::from(&req.bundle_path);
    if !bundle_path.exists() {
        return err(
            StatusCode::BAD_REQUEST,
            "file_not_found",
            format!("bundle not found: {}", req.bundle_path),
        )
        .into_response();
    }

    // Read manifest to determine corpus ID.
    let manifest = match ministr_core::bundle::read_manifest(&bundle_path) {
        Ok(m) => m,
        Err(e) => {
            return err(StatusCode::BAD_REQUEST, "invalid_bundle", e).into_response();
        }
    };

    let corpus_id = format!(
        "import-{}",
        &ministr_core::bundle::compute_bundle_version(&manifest.corpus_roots)[..8]
    );
    let corpus_dir = state
        .registry
        .config()
        .data_dir
        .join("corpora")
        .join(&corpus_id);

    match ministr_core::bundle::import_bundle(&bundle_path, &corpus_dir) {
        Ok(imported_manifest) => Json(ministr_api::corpus::ImportBundleResponse {
            corpus_id,
            manifest: convert::bundle_manifest(&imported_manifest),
        })
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "import_failed", e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Prefetch
// ---------------------------------------------------------------------------

/// Number of nearest neighbours the topical prefetch strategy asks the
/// vector index for after each read. 8 is a balance between covering the
/// local neighbourhood and not thrashing the cache.
const TOPICAL_PREFETCH_K: usize = 8;

/// Run sequential + structural + topical prefetch strategies after a read.
///
/// Runs in a spawned task so the read response isn't delayed. Four phases:
/// 1. **Sequential** — the next section in document order.
/// 2. **Structural** — ±2 sibling sections around the current read position.
/// 3. **Topic tracker feed** — re-embed the read section and update the
///    engine's rolling topic vector.
/// 4. **Topical** — query HNSW with the updated topic vector and pre-warm
///    the top section-resolution neighbours.
///
/// Cross-session prefetch requires per-section co-access analytics that
/// the storage layer doesn't yet track; left as a future strategy.
async fn trigger_prefetch(
    section_id: &str,
    storage: &ministr_core::storage::SqliteStorage,
    prefetch: &tokio::sync::Mutex<ministr_core::session::prefetch::PrefetchEngine>,
    embedder: &dyn ministr_core::embedding::Embedder,
    index: &dyn ministr_core::index::VectorIndex,
) {
    use ministr_core::storage::Storage;
    use ministr_core::types::SectionId;

    let sid = SectionId(section_id.to_string());

    // ── Sequential ────────────────────────────────────────────────────
    let next_section = storage.get_next_section(&sid).await.unwrap_or(None);
    let claims_count = if let Some(ref next) = next_section {
        storage.list_claims(&next.id).await.map(|c| c.len()).ok()
    } else {
        None
    };
    let doc_record = storage.get_document_for_section(&sid).await.ok().flatten();
    {
        let mut pf = prefetch.lock().await;
        pf.advance_turn();
        pf.prefetch_sequential(next_section, claims_count);
    }

    // ── Structural ────────────────────────────────────────────────────
    if let Some(ref doc) = doc_record
        && let Ok(all_sections) = storage.list_sections(&doc.id).await
    {
        let current_pos = all_sections.iter().position(|s| s.id.0 == section_id);
        if let Some(pos) = current_pos {
            let start = pos.saturating_sub(2);
            let end = (pos + 3).min(all_sections.len());
            let siblings: Vec<_> = all_sections[start..end]
                .iter()
                .filter(|s| s.id.0 != section_id)
                .cloned()
                .collect();
            let mut claims_counts = std::collections::HashMap::new();
            for s in &siblings {
                if let Ok(claims) = storage.list_claims(&s.id).await {
                    claims_counts.insert(s.id.0.clone(), claims.len());
                }
            }
            let mut pf = prefetch.lock().await;
            pf.prefetch_structural(siblings, &claims_counts);
        }
    }

    // ── Topic feed + topical ──────────────────────────────────────────
    // Re-embed the read section to feed the running topic vector and to
    // query for topically-similar candidates. If anything on this path
    // fails (no section, embed failure, empty topic vector, search failure)
    // we silently skip — topical prefetch is an optimization, not a
    // correctness requirement.
    let Ok(Some(current)) = storage.get_section(&sid).await else {
        return;
    };
    if current.text.is_empty() {
        return;
    }
    let section_vec = match embedder.embed(&[current.text.as_str()]) {
        Ok(mut vecs) if !vecs.is_empty() => vecs.remove(0),
        _ => return,
    };

    // Feed the topic tracker + read back the current topic vector.
    let topic_vec = {
        let mut pf = prefetch.lock().await;
        pf.record_topic_access(section_vec);
        pf.topic_vector()
    };
    let Some(topic_vec) = topic_vec else { return };

    let Ok(results) = index.search_knn(&topic_vec, TOPICAL_PREFETCH_K) else {
        return;
    };

    // Keep only section-resolution hits, strip the current section, and
    // skip anything the engine already has cached.
    let mut candidate_ids: Vec<String> = Vec::new();
    for r in results {
        let Some(vid) = ministr_core::types::VectorId::parse(&r.id) else {
            continue;
        };
        if vid.resolution() != ministr_core::types::Resolution::Section {
            continue;
        }
        let cid = vid.content_id().to_string();
        if cid == section_id {
            continue;
        }
        candidate_ids.push(cid);
    }
    if candidate_ids.is_empty() {
        return;
    }

    let mut candidates: Vec<ministr_core::storage::SectionRecord> = Vec::new();
    let mut claims_counts = std::collections::HashMap::new();
    for cid in &candidate_ids {
        let section_id_typed = SectionId(cid.clone());
        if let Ok(Some(section)) = storage.get_section(&section_id_typed).await {
            if let Ok(claims) = storage.list_claims(&section.id).await {
                claims_counts.insert(section.id.0.clone(), claims.len());
            }
            candidates.push(section);
        }
    }

    if !candidates.is_empty() {
        let mut pf = prefetch.lock().await;
        pf.prefetch_topical(candidates, &claims_counts);
    }
}

async fn prefetch_metrics(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let pf = handle.prefetch.lock().await;
    let metrics = pf.metrics();
    let size = pf.cache().len();
    let capacity = pf.cache().capacity();
    Json(convert::prefetch_metrics(&metrics, size, capacity)).into_response()
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// Generate a unique session ID from the current timestamp.
fn generate_session_id() -> String {
    let mut hasher = Sha256::new();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.update(nanos.to_le_bytes());
    // Mix in pointer entropy to avoid collisions on fast successive calls.
    let entropy: u64 = std::ptr::from_ref(&hasher) as u64;
    hasher.update(entropy.to_le_bytes());
    let hash = hasher.finalize();
    format!(
        "sess-{:x}",
        &hash[..8]
            .iter()
            .fold(0u64, |acc, &b| (acc << 8) | u64::from(b))
    )
}

async fn create_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<CreateSessionRequest>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);

    let session_id = generate_session_id();
    let budget_tokens = req.budget_tokens.unwrap_or(100_000);
    let data_dir = handle.data_dir.clone();

    let mut sessions = handle.sessions.lock().await;
    let budget_config = ministr_core::session::UsageConfig {
        max_context_tokens: budget_tokens,
        ..ministr_core::session::UsageConfig::default()
    };
    sessions.get_or_create(&session_id, Some(budget_config), AccessMode::ReadWrite);
    drop(sessions);

    // Persist the new session.
    let db_path = data_dir.join("sessions.db");
    if let Err(e) = crate::persistence::save_session(
        &db_path,
        &id,
        &session_id,
        budget_tokens,
        0,
        &std::collections::BTreeMap::new(),
        &[],
    ) {
        tracing::warn!(error = %e, "failed to persist session");
    }

    (
        StatusCode::CREATED,
        Json(CreateSessionResponse { session_id }),
    )
        .into_response()
}

async fn session_usage(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);

    let mut sessions = handle.sessions.lock().await;
    // If session exists in memory but budget is 0, try reconstructing from
    // persisted delivered items (handles daemon restart with stale budget).
    if let Some(entry) = sessions.get_session_mut(&sid) {
        let status = entry.budget.usage_status();
        if status.tokens_used == 0 && entry.session.delivered_count() > 0 {
            // Budget was reset (daemon restart) but session has deliveries.
            // Replay delivered items to reconstruct the budget.
            for item in entry.session.delivered_items() {
                let _ = entry
                    .budget
                    .record_tokens(item.content_id.as_ref(), item.token_count);
            }
        }
        let status = entry.budget.usage_status();
        return Json(convert::usage_status(&status)).into_response();
    }

    // Session not in memory — try loading from SQLite.
    let session_id = ministr_core::session::SessionId::from(sid.clone());
    if let Ok(Some(restored)) = handle.storage.load_session(&session_id).await {
        let entry = sessions.get_or_create(&sid, None, AccessMode::ReadWrite);
        for item in restored.delivered_items() {
            let _ = entry
                .budget
                .record_tokens(item.content_id.as_ref(), item.token_count);
        }
        entry.session = restored;
        let status = entry.budget.usage_status();
        return Json(convert::usage_status(&status)).into_response();
    }

    err(
        StatusCode::NOT_FOUND,
        "session_not_found",
        format!("session {sid} not found"),
    )
    .into_response()
}

async fn destroy_session(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let data_dir = handle.data_dir.clone();

    let mut sessions = handle.sessions.lock().await;
    if sessions.remove_session(&sid).is_some() {
        drop(sessions);
        // Remove persisted session.
        let db_path = data_dir.join("sessions.db");
        if let Err(e) = crate::persistence::delete_session(&db_path, &id, &sid) {
            tracing::warn!(error = %e, "failed to delete persisted session");
        }
        StatusCode::NO_CONTENT.into_response()
    } else {
        err(
            StatusCode::NOT_FOUND,
            "session_not_found",
            format!("session {sid} not found"),
        )
        .into_response()
    }
}

/// Remove all sessions for a corpus (e.g. on proxy reconnect).
async fn clear_sessions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);
    let data_dir = handle.data_dir.clone();

    let mut sessions = handle.sessions.lock().await;
    let ids: Vec<String> = sessions.session_ids();
    let count = ids.len();
    for sid in &ids {
        sessions.remove_session(sid);
    }
    drop(sessions);

    // Remove persisted sessions.
    let db_path = data_dir.join("sessions.db");
    for sid in &ids {
        if let Err(e) = crate::persistence::delete_session(&db_path, &id, sid) {
            tracing::warn!(error = %e, session_id = %sid, "failed to delete persisted session");
        }
    }

    tracing::info!(corpus_id = %id, cleared = count, "cleared all sessions");
    StatusCode::NO_CONTENT.into_response()
}

async fn drop_content(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
    Json(req): Json<ministr_api::session::DropRequest>,
) -> impl IntoResponse {
    let handle = get_corpus!(&state, &id);

    let mut sessions = handle.sessions.lock().await;
    match sessions.get_session_mut(&sid) {
        Some(entry) => {
            let mut dropped = Vec::new();
            let mut not_found = Vec::new();

            for id_str in &req.content_ids {
                let content_id = ministr_core::types::ContentId(id_str.clone());
                if entry.session.remove_delivered(&content_id).is_some() {
                    entry.budget.force_evict(id_str);
                    dropped.push(id_str.clone());
                } else {
                    not_found.push(id_str.clone());
                }
            }

            Json(ministr_api::session::DropResponse { dropped, not_found }).into_response()
        }
        None => err(
            StatusCode::NOT_FOUND,
            "session_not_found",
            format!("session {sid} not found"),
        )
        .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Admin
// ---------------------------------------------------------------------------

async fn daemon_status(State(state): State<AppState>) -> impl IntoResponse {
    let corpora = state.registry.list().await;
    let total_sessions: usize = corpora.iter().map(|c| c.active_sessions).sum();
    Json(DaemonStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: state.uptime_secs(),
        memory_mb: ministr_core::mem_profile::rss_mb().unwrap_or(0.0),
        model: state.registry.config().default_model.clone(),
        model_dimension: state.registry.embedder().dimension(),
        corpora,
        log_path: None,
        total_sessions,
        // Autostart is a desktop-only concept; the headless daemon doesn't
        // know whether the tray app is configured to launch at login.
        autostart_enabled: None,
    })
}

#[cfg(test)]
mod tests {
    //! Tests for the router split.
    //!
    //! We distinguish *handler*-404s (request reached the handler and it
    //! decided the resource doesn't exist) from *routing*-404s (no route
    //! registered for this method+path). For the negative cases we treat
    //! BOTH 404 and 405 (Method Not Allowed) as "route does not serve this
    //! combination" — axum 0.8 returns 405 when a path-shape exists but the
    //! method doesn't match, which can happen because path parameters like
    //! `{id}` swallow whole segments. The positive side uses routes whose
    //! handlers return 200 on an empty registry (e.g. GET /api/v1/corpora →
    //! empty list).
    use super::*;
    use axum::body::Body;
    use http::StatusCode;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        use ministr_core::config::MinistrConfig;

        struct FixedEmbedder;
        impl ministr_core::embedding::Embedder for FixedEmbedder {
            fn embed(
                &self,
                texts: &[&str],
            ) -> Result<Vec<Vec<f32>>, ministr_core::error::IndexError> {
                Ok(texts.iter().map(|_| vec![0.0; 384]).collect())
            }
            fn dimension(&self) -> usize {
                384
            }
        }

        let embedder: Arc<dyn ministr_core::embedding::Embedder> = Arc::new(FixedEmbedder);
        let registry = crate::registry::CorpusRegistry::new(embedder, MinistrConfig::default());
        crate::state::AppState::new(registry)
    }

    async fn status_of(router: &Router, method: &str, uri: &str) -> StatusCode {
        let req = http::Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        router.clone().oneshot(req).await.unwrap().status()
    }

    /// "Not routed" — request did not reach any handler matching method+path.
    fn unrouted(status: StatusCode) -> bool {
        status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED
    }

    #[test]
    fn inject_github_app_token_rewrites_https_urls() {
        let url = super::inject_github_app_token(
            "https://github.com/anthropics/ministr.git",
            "ghs_abc123",
        )
        .expect("rewrite succeeds");
        assert_eq!(
            url,
            "https://x-access-token:ghs_abc123@github.com/anthropics/ministr.git"
        );
    }

    #[test]
    fn inject_github_app_token_rejects_ssh_urls() {
        let err = super::inject_github_app_token("git@github.com:owner/repo.git", "ghs_x")
            .expect_err("SSH URLs cannot carry installation tokens");
        assert!(err.contains("https://"), "got {err}");
    }

    #[test]
    fn inject_github_app_token_rejects_urls_with_existing_credentials() {
        let err = super::inject_github_app_token(
            "https://alice:pat-1234@github.com/owner/repo.git",
            "ghs_x",
        )
        .expect_err("PAT-in-URL + App token would conflict");
        assert!(err.contains("credentials"), "got {err}");
    }

    #[tokio::test]
    async fn read_router_serves_known_read_paths() {
        let app = corpora_read_router(test_state());
        // GET /api/v1/corpora → handler returns 200 with empty list.
        assert_eq!(
            status_of(&app, "GET", "/api/v1/corpora").await,
            StatusCode::OK,
        );
        // GET /api/v1/status → handler returns 200.
        assert_eq!(
            status_of(&app, "GET", "/api/v1/status").await,
            StatusCode::OK,
        );
    }

    #[tokio::test]
    async fn read_router_rejects_write_methods() {
        let app = corpora_read_router(test_state());
        // POST /api/v1/corpora — register is not on read router.
        assert!(unrouted(status_of(&app, "POST", "/api/v1/corpora").await));
        // DELETE /api/v1/corpora/x — unregister is not on read router.
        assert!(unrouted(status_of(&app, "DELETE", "/api/v1/corpora/x").await));
        // POST /api/v1/corpora/x/clone — clone is not on read router.
        assert!(unrouted(
            status_of(&app, "POST", "/api/v1/corpora/x/clone").await,
        ));
        // POST /api/v1/corpora/import — bundle import is not on read router.
        assert!(unrouted(
            status_of(&app, "POST", "/api/v1/corpora/import").await,
        ));
        // GET /activity — observability is not on read router.
        assert!(unrouted(status_of(&app, "GET", "/activity").await));
    }

    #[tokio::test]
    async fn write_router_rejects_read_paths() {
        let app = corpora_write_router(test_state());
        // GET /api/v1/corpora — list is read-only, not on write router.
        assert!(unrouted(status_of(&app, "GET", "/api/v1/corpora").await));
        // GET /api/v1/status — read-only.
        assert!(unrouted(status_of(&app, "GET", "/api/v1/status").await));
        // POST /api/v1/corpora/x/survey — read-only.
        assert!(unrouted(
            status_of(&app, "POST", "/api/v1/corpora/x/survey").await,
        ));
        // POST /api/v1/corpora/import — bundle router, not write.
        assert!(unrouted(
            status_of(&app, "POST", "/api/v1/corpora/import").await,
        ));
    }

    #[tokio::test]
    async fn bundle_router_serves_bundle_paths() {
        let app = corpora_bundle_router(test_state());
        // POST /api/v1/corpora/import — handler reaches its body parser
        // (rejects empty body, but the response is from the handler, so the
        // path IS routed — manifests as 400/415 rather than routing-404).
        let s = status_of(&app, "POST", "/api/v1/corpora/import").await;
        assert!(!unrouted(s), "POST .../import should be routed, got {s}");
        // GET /api/v1/corpora — not on bundle router.
        assert!(unrouted(status_of(&app, "GET", "/api/v1/corpora").await));
    }

    #[tokio::test]
    async fn observability_router_serves_observability_only() {
        let app = observability_router(test_state());
        assert_eq!(
            status_of(&app, "GET", "/activity").await,
            StatusCode::OK,
        );
        assert_eq!(
            status_of(&app, "GET", "/coherence-events").await,
            StatusCode::OK,
        );
        // No corpus paths on observability router.
        assert!(unrouted(status_of(&app, "GET", "/api/v1/corpora").await));
        assert!(unrouted(status_of(&app, "POST", "/api/v1/corpora/import").await));
    }

    #[tokio::test]
    async fn ask_router_isolated() {
        let app = corpora_ask_router(test_state());
        // The ask path is routed. Handler may fail (no `claude` CLI on the
        // test runner), but routing-wise the path exists. Method GET should
        // therefore return 405 (path exists, method doesn't match).
        assert!(unrouted(status_of(&app, "GET", "/api/v1/corpora/x/ask").await));
        // No other paths on ask router.
        assert!(unrouted(status_of(&app, "GET", "/api/v1/corpora").await));
    }

    #[tokio::test]
    async fn composed_router_includes_every_sub_router() {
        // Regression: if a sub-builder is ever dropped from `router()`, at
        // least one of these probes will surface routing-404 on a path that
        // the composed router should serve.
        let app = router(test_state());

        // Each probe checks routing by method/path; status code is allowed
        // to be anything except a routing failure.
        for (method, path, label) in &[
            ("GET", "/api/v1/corpora", "read: list"),
            ("GET", "/api/v1/status", "read: status"),
            ("POST", "/api/v1/corpora/import", "bundle: import"),
            ("GET", "/activity", "observability: activity"),
            ("GET", "/coherence-events", "observability: coherence"),
        ] {
            let s = status_of(&app, method, path).await;
            assert!(
                !unrouted(s),
                "composed router missing {label} ({method} {path}) — got {s}"
            );
        }
    }
}
