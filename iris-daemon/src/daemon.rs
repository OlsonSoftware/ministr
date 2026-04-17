//! HTTP daemon on a Unix domain socket.
//!
//! Exposes the iris daemon API via axum at `~/.iris/irisd.sock`.
//! All handlers delegate to [`QueryService`] via the [`CorpusRegistry`].

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_core::Stream;
use tokio::net::UnixListener;
use tracing::info;

use iris_api::ApiError;
use iris_api::activity::ActivityResponse;
use iris_api::coherence::CoherenceEventsResponse;
use iris_api::corpus::{ListCorporaResponse, RegisterCorpusRequest, RegisterCorpusResponse};
use iris_api::query;
use iris_api::session::{CreateSessionRequest, CreateSessionResponse};
use iris_api::status::DaemonStatus;
use iris_core::session::AccessMode;
use iris_core::storage::{Storage as _, SymbolFilter};
use iris_core::types::RelationType;
use sha2::{Digest, Sha256};

use crate::activity::record as record_activity;
use crate::convert;
use crate::state::{ACTIVITY_BUFFER_CAPACITY, AppState, COHERENCE_BUFFER_CAPACITY};

/// Build the daemon API router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/corpora", post(register_corpus).get(list_corpora))
        .route(
            "/api/v1/corpora/{id}",
            get(corpus_status).delete(unregister_corpus),
        )
        .route("/api/v1/corpora/{id}/survey", post(survey))
        .route("/api/v1/corpora/{id}/symbols", post(symbols))
        .route("/api/v1/corpora/{id}/definition/{sym}", get(definition))
        .route("/api/v1/corpora/{id}/references/{sym}", get(references))
        .route("/api/v1/corpora/{id}/read/{section}", get(read_section))
        .route("/api/v1/corpora/{id}/extract", post(extract))
        .route("/api/v1/corpora/{id}/toc", post(toc))
        .route("/api/v1/corpora/{id}/related", post(related))
        .route("/api/v1/corpora/{id}/bridge", post(bridge))
        .route("/api/v1/corpora/{id}/compress", post(compress_content))
        .route("/api/v1/corpora/{id}/ask", post(ask_handler))
        .route("/api/v1/corpora/{id}/export", post(export_bundle))
        .route("/api/v1/corpora/{id}/progress", get(ingestion_progress))
        .route("/api/v1/corpora/{id}/coherence", get(coherence_stream))
        .route("/api/v1/corpora/{id}/prefetch", get(prefetch_metrics))
        .route("/api/v1/corpora/import", post(import_bundle))
        .route(
            "/api/v1/corpora/{id}/sessions",
            post(create_session).delete(clear_sessions),
        )
        .route(
            "/api/v1/corpora/{id}/sessions/{sid}/budget",
            get(session_budget),
        )
        .route(
            "/api/v1/corpora/{id}/sessions/{sid}/read/{section}",
            get(session_read_section),
        )
        .route(
            "/api/v1/corpora/{id}/sessions/{sid}/evicted",
            post(evict_content),
        )
        .route(
            "/api/v1/corpora/{id}/sessions/{sid}",
            axum::routing::delete(destroy_session),
        )
        .route("/api/v1/status", get(daemon_status))
        .route("/activity", get(recent_activity))
        .route("/coherence-events", get(recent_coherence_events))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            record_activity,
        ))
        .with_state(state)
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

/// Start the daemon listener on the Unix domain socket.
///
/// Writes a PID file for process liveness detection and removes stale
/// sockets from crashed predecessors. On graceful shutdown, cleans up
/// both the socket and PID file.
///
/// # Errors
///
/// Returns an error if the socket cannot be bound or another daemon is running.
pub async fn start(state: AppState) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = iris_api::daemon_socket_path();
    let pid_path = iris_api::daemon_pid_path();

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Startup resilience: detect stale socket from a crashed predecessor.
    if socket_path.exists() {
        if is_daemon_process_alive(&pid_path) {
            return Err("another iris daemon is already running".into());
        }
        tracing::warn!("removing stale socket from crashed daemon");
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&pid_path);
    }

    let listener = UnixListener::bind(&socket_path)?;

    // Write PID file for liveness detection by proxies and future launches.
    let pid = std::process::id();
    std::fs::write(&pid_path, pid.to_string())?;
    info!(path = %socket_path.display(), pid, "daemon listening on UDS");

    // Graceful shutdown on ctrl-c or SIGTERM.
    let shutdown = shutdown_signal();
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await?;

    info!("daemon shutting down gracefully");
    let _ = std::fs::remove_file(&socket_path);
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
pub async fn serve(
    state: AppState,
    listener: UnixListener,
) -> Result<(), Box<dyn std::error::Error>> {
    axum::serve(listener, router(state)).await?;
    Ok(())
}

/// Check if a daemon is actually listening on the socket.
///
/// Attempts a TCP-level connect to the UDS. If it succeeds, a live daemon
/// owns the socket. This avoids `unsafe` process checks via `kill(pid, 0)`.
fn is_daemon_process_alive(pid_path: &std::path::Path) -> bool {
    // If no PID file exists, the socket is certainly stale.
    if !pid_path.exists() {
        return false;
    }
    // Try connecting to the socket — if it succeeds, a daemon is alive.
    let socket_path = iris_api::daemon_socket_path();
    std::os::unix::net::UnixStream::connect(socket_path).is_ok()
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

/// Resolve a corpus ID to its handle, returning a 404 response on failure.
/// The caller must hold the returned guard for the duration of use.
macro_rules! get_corpus {
    ($state:expr, $id:expr) => {
        match $state.registry.get($id).await {
            Ok(guard) => guard,
            Err(e) => return err(StatusCode::NOT_FOUND, "not_found", e).into_response(),
        }
    };
}

// ---------------------------------------------------------------------------
// Corpus management
// ---------------------------------------------------------------------------

async fn register_corpus(
    State(state): State<AppState>,
    Json(req): Json<RegisterCorpusRequest>,
) -> impl IntoResponse {
    match state.registry.register(&req.paths).await {
        Ok((corpus_id, indexing_started)) => Json(RegisterCorpusResponse {
            corpus_id,
            indexing_started,
        })
        .into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, "register_failed", e).into_response(),
    }
}

async fn list_corpora(State(state): State<AppState>) -> impl IntoResponse {
    Json(ListCorporaResponse {
        corpora: state.registry.list().await,
    })
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
) -> impl IntoResponse {
    match state.registry.unregister(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(StatusCode::NOT_FOUND, "not_found", e).into_response(),
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
    let guard = get_corpus!(&state, &id);
    let top_k = req.top_k.unwrap_or(10);
    match guard[&id].service.survey(&req.query, top_k).await {
        Ok(results) => Json(query::SurveyResponse {
            results: results.into_iter().map(convert::survey_result).collect(),
            deduplicated_count: None,
            budget_status: None,
        })
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn symbols(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<query::SymbolsRequest>,
) -> impl IntoResponse {
    let _permit = state.query_semaphore.acquire().await;
    let guard = get_corpus!(&state, &id);
    let limit = req.limit.unwrap_or(20);
    let filter = SymbolFilter {
        name: Some(req.query),
        name_exact: None,
        kind: req.kind,
        visibility: req.visibility,
        module: req.module,
        file_path: None,
    };
    match guard[&id].service.search_symbols(&filter).await {
        Ok(records) => Json(query::SymbolsResponse {
            symbols: records
                .into_iter()
                .take(limit)
                .map(convert::symbol_from_record)
                .collect(),
        })
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn definition(
    State(state): State<AppState>,
    Path((id, sym)): Path<(String, String)>,
) -> impl IntoResponse {
    let guard = get_corpus!(&state, &id);
    match guard[&id].service.get_symbol_definition(&sym).await {
        Ok(def) => Json(convert::symbol_definition(def)).into_response(),
        Err(e) => err(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    }
}

async fn references(
    State(state): State<AppState>,
    Path((id, sym)): Path<(String, String)>,
) -> impl IntoResponse {
    let guard = get_corpus!(&state, &id);
    match guard[&id].service.get_symbol_references(&sym, None).await {
        Ok(refs) => Json(query::ReferencesResponse {
            references: refs.into_iter().map(convert::symbol_reference).collect(),
        })
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn read_section(
    State(state): State<AppState>,
    Path((id, section)): Path<(String, String)>,
) -> impl IntoResponse {
    let guard = get_corpus!(&state, &id);
    let handle = &guard[&id];

    // Check prefetch cache for a warm hit.
    let warm_detail = {
        let mut prefetch = handle.prefetch.lock().await;
        prefetch
            .try_serve(&section)
            .map(|entry| iris_core::service::SectionDetail {
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
            let section_clone = section.clone();
            drop(guard);

            // Spawn background prefetch (don't block the response).
            tokio::spawn(async move {
                trigger_prefetch(&section_clone, &storage, &prefetch).await;
            });
            Json(convert::section_detail(detail)).into_response()
        }
        Err(e) => err(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    }
}

/// Session-aware read: records delivery in the session shadow + budget tracker.
///
/// Used by the MCP proxy so that `iris_budget` reflects actual token usage.
async fn session_read_section(
    State(state): State<AppState>,
    Path((id, sid, section)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let guard = get_corpus!(&state, &id);
    let handle = &guard[&id];

    // Check prefetch cache for a warm hit.
    let warm_detail = {
        let mut prefetch = handle.prefetch.lock().await;
        prefetch
            .try_serve(&section)
            .map(|entry| iris_core::service::SectionDetail {
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
                let token_count = iris_core::token::count_tokens(&detail.text);
                let content_id = iris_core::types::ContentId(section.clone());
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
                entry.session.record_delivery(
                    &content_id,
                    iris_core::types::Resolution::Section,
                    token_count,
                    turn,
                    content_hash,
                );
                let _ = entry.budget.record_tokens(&section, token_count);
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
            let section_clone = section.clone();
            drop(guard);

            // Spawn background prefetch (don't block the response).
            tokio::spawn(async move {
                trigger_prefetch(&section_clone, &storage, &prefetch).await;
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
    let guard = get_corpus!(&state, &id);
    match guard[&id]
        .service
        .extract_claims(&req.section_id, req.query.as_deref())
        .await
    {
        Ok(claims) => Json(query::ExtractResponse {
            claims: claims.into_iter().map(convert::claim_result).collect(),
        })
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn toc(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<query::TocRequest>,
) -> impl IntoResponse {
    let guard = get_corpus!(&state, &id);
    let offset = req.offset.unwrap_or(0);
    let limit = req.limit.unwrap_or(100);
    match guard[&id].service.toc(req.document_id.as_deref()).await {
        Ok(entries) => {
            let total = entries.len();
            Json(query::TocResponse {
                entries: entries
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .map(convert::toc_entry)
                    .collect(),
                total,
            })
            .into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn related(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<query::RelatedRequest>,
) -> impl IntoResponse {
    let guard = get_corpus!(&state, &id);
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
    match guard[&id]
        .service
        .related_claims(&req.claim_id, relation_types.as_deref())
        .await
    {
        Ok(claims) => Json(query::RelatedResponse {
            claims: claims.into_iter().map(convert::related_claim).collect(),
        })
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

async fn bridge(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<query::BridgeRequest>,
) -> impl IntoResponse {
    let guard = get_corpus!(&state, &id);
    let limit = req.limit.unwrap_or(50);
    match guard[&id]
        .service
        .query_bridges(
            req.query.as_deref(),
            req.kind.as_deref(),
            req.source_language.as_deref(),
            None,
        )
        .await
    {
        Ok(links) => Json(query::BridgeResponse {
            links: links
                .into_iter()
                .take(limit)
                .map(convert::bridge_link)
                .collect(),
        })
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Compress
// ---------------------------------------------------------------------------

async fn compress_content(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<iris_api::session::CompressRequest>,
) -> impl IntoResponse {
    let _permit = state.query_semaphore.acquire().await;
    let guard = get_corpus!(&state, &id);
    match guard[&id].service.compress_content(&req.content_ids).await {
        Ok(items) => Json(iris_api::session::CompressResponse {
            summaries: items.into_iter().map(convert::compressed_item).collect(),
        })
        .into_response(),
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
    let guard = get_corpus!(&state, &id);
    let handle = &guard[&id];

    match crate::ask::ask(
        &req.query,
        &handle.service,
        &handle.storage,
        state.inference.as_ref(),
    )
    .await
    {
        Ok(result) => Json(query::AskResponse {
            answer: result.answer,
            source_ids: result.source_ids,
            cached: result.cached,
            model: result.model,
        })
        .into_response(),
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
    progress: Arc<iris_core::ingestion::IngestionProgress>,
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
            let event = iris_api::corpus::IngestionProgressEvent {
                status: status.to_string(),
                phase: progress.phase().as_str().to_string(),
                files_total: progress.files_total(),
                files_done: progress.files_done(),
                sections_done: progress.sections_done(),
                embeddings_total: progress.embeddings_total(),
                embeddings_done: progress.embeddings_done(),
                current_file: progress.current_file(),
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

    let output_path = data_dir.join(format!("{id}.iris-index"));
    let manifest = iris_core::bundle::BundleManifest {
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

    match iris_core::bundle::export_bundle(&data_dir, &output_path, &manifest) {
        Ok(path) => {
            // Re-read manifest from the exported bundle for accurate counts.
            let final_manifest = iris_core::bundle::read_manifest(&path).unwrap_or(manifest);
            Json(iris_api::corpus::ExportBundleResponse {
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
    Json(req): Json<iris_api::corpus::ImportBundleRequest>,
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
    let manifest = match iris_core::bundle::read_manifest(&bundle_path) {
        Ok(m) => m,
        Err(e) => {
            return err(StatusCode::BAD_REQUEST, "invalid_bundle", e).into_response();
        }
    };

    let corpus_id = format!(
        "import-{}",
        &iris_core::bundle::compute_bundle_version(&manifest.corpus_roots)[..8]
    );
    let corpus_dir = state
        .registry
        .config()
        .data_dir
        .join("corpora")
        .join(&corpus_id);

    match iris_core::bundle::import_bundle(&bundle_path, &corpus_dir) {
        Ok(imported_manifest) => Json(iris_api::corpus::ImportBundleResponse {
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

/// Run sequential + structural prefetch strategies after a read operation.
///
/// Runs in a spawned task so the read response isn't delayed.
async fn trigger_prefetch(
    section_id: &str,
    storage: &iris_core::storage::SqliteStorage,
    prefetch: &tokio::sync::Mutex<iris_core::session::prefetch::PrefetchEngine>,
) {
    use iris_core::storage::Storage;
    use iris_core::types::SectionId;

    let sid = SectionId(section_id.to_string());

    // Sequential: next section + parent document summary.
    let next_section = storage.get_next_section(&sid).await.unwrap_or(None);
    let claims_count = if let Some(ref next) = next_section {
        storage.list_claims(&next.id).await.map(|c| c.len()).ok()
    } else {
        None
    };
    let doc_record = storage.get_document_for_section(&sid).await.ok().flatten();
    let doc_summary = doc_record
        .as_ref()
        .and_then(|doc| doc.summary.as_ref().map(|s| (doc.id.0.clone(), s.clone())));

    {
        let mut pf = prefetch.lock().await;
        pf.prefetch_sequential(next_section, doc_summary, claims_count);
    }

    // Structural: sibling sections from the same document.
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
}

async fn prefetch_metrics(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let guard = get_corpus!(&state, &id);
    let handle = &guard[&id];
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
    let guard = get_corpus!(&state, &id);
    let handle = &guard[&id];

    let session_id = generate_session_id();
    let budget_tokens = req.budget_tokens.unwrap_or(100_000);
    let data_dir = handle.data_dir.clone();

    let mut sessions = handle.sessions.lock().await;
    let budget_config = iris_core::session::BudgetConfig {
        max_context_tokens: budget_tokens,
        ..iris_core::session::BudgetConfig::default()
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

async fn session_budget(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
) -> impl IntoResponse {
    let guard = get_corpus!(&state, &id);
    let handle = &guard[&id];

    let mut sessions = handle.sessions.lock().await;
    // If session exists in memory but budget is 0, try reconstructing from
    // persisted delivered items (handles daemon restart with stale budget).
    if let Some(entry) = sessions.get_session_mut(&sid) {
        let status = entry.budget.budget_status();
        if status.tokens_used == 0 && entry.session.delivered_count() > 0 {
            // Budget was reset (daemon restart) but session has deliveries.
            // Replay delivered items to reconstruct the budget.
            for item in entry.session.delivered_items() {
                let _ = entry
                    .budget
                    .record_tokens(item.content_id.as_ref(), item.token_count);
            }
        }
        let status = entry.budget.budget_status();
        return Json(convert::budget_status(&status)).into_response();
    }

    // Session not in memory — try loading from SQLite.
    let session_id = iris_core::session::SessionId::from(sid.clone());
    if let Ok(Some(restored)) = handle.storage.load_session(&session_id).await {
        let entry = sessions.get_or_create(&sid, None, AccessMode::ReadWrite);
        for item in restored.delivered_items() {
            let _ = entry
                .budget
                .record_tokens(item.content_id.as_ref(), item.token_count);
        }
        entry.session = restored;
        let status = entry.budget.budget_status();
        return Json(convert::budget_status(&status)).into_response();
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
    let guard = get_corpus!(&state, &id);
    let handle = &guard[&id];
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
    let guard = get_corpus!(&state, &id);
    let handle = &guard[&id];
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

async fn evict_content(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
    Json(req): Json<iris_api::session::EvictRequest>,
) -> impl IntoResponse {
    let guard = get_corpus!(&state, &id);
    let handle = &guard[&id];

    let mut sessions = handle.sessions.lock().await;
    match sessions.get_session_mut(&sid) {
        Some(entry) => {
            let mut evicted = Vec::new();
            let mut not_found = Vec::new();

            for id_str in &req.content_ids {
                let content_id = iris_core::types::ContentId(id_str.clone());
                if entry.session.remove_delivered(&content_id).is_some() {
                    entry.budget.force_evict(id_str);
                    evicted.push(id_str.clone());
                } else {
                    not_found.push(id_str.clone());
                }
            }

            Json(iris_api::session::EvictResponse { evicted, not_found }).into_response()
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
        memory_mb: iris_core::mem_profile::rss_mb().unwrap_or(0.0),
        model: state.registry.config().default_model.clone(),
        model_dimension: state.registry.embedder().dimension(),
        corpora,
        log_path: None,
        total_sessions,
    })
}
