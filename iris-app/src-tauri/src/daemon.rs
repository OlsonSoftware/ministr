//! HTTP daemon on a Unix domain socket.
//!
//! Exposes the iris daemon API via axum at `~/.iris/irisd.sock`.
//! All handlers delegate to [`QueryService`] via the [`CorpusRegistry`].

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::net::UnixListener;
use tracing::info;

use iris_api::ApiError;
use iris_api::corpus::{ListCorporaResponse, RegisterCorpusRequest, RegisterCorpusResponse};
use iris_api::query;
use iris_api::session::{CreateSessionRequest, CreateSessionResponse};
use iris_api::status::DaemonStatus;
use iris_core::session::AccessMode;
use iris_core::storage::SymbolFilter;
use iris_core::types::RelationType;
use sha2::{Digest, Sha256};

use crate::convert;
use crate::state::AppState;

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
        .route("/api/v1/corpora/{id}/sessions", post(create_session))
        .route(
            "/api/v1/corpora/{id}/sessions/{sid}/budget",
            get(session_budget),
        )
        .route(
            "/api/v1/corpora/{id}/sessions/{sid}",
            axum::routing::delete(destroy_session),
        )
        .route("/api/v1/status", get(daemon_status))
        .with_state(state)
}

/// Start the daemon listener on the Unix domain socket.
pub async fn start(state: AppState) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = iris_api::daemon_socket_path();
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    info!(path = %socket_path.display(), "daemon listening on UDS");

    axum::serve(listener, router(state)).await?;
    let _ = std::fs::remove_file(&socket_path);
    Ok(())
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
    match guard[&id].service.read_section(&section).await {
        Ok(detail) => Json(convert::section_detail(detail)).into_response(),
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

    let mut sessions = handle.sessions.lock().await;
    let budget_config = iris_core::session::BudgetConfig {
        max_context_tokens: budget_tokens,
        ..iris_core::session::BudgetConfig::default()
    };
    sessions.get_or_create(&session_id, Some(budget_config), AccessMode::ReadWrite);

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

    let sessions = handle.sessions.lock().await;
    match sessions.get_session(&sid) {
        Some(entry) => {
            let status = entry.budget.budget_status();
            Json(convert::budget_status(&status)).into_response()
        }
        None => err(
            StatusCode::NOT_FOUND,
            "session_not_found",
            format!("session {sid} not found"),
        )
        .into_response(),
    }
}

async fn destroy_session(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
) -> impl IntoResponse {
    let guard = get_corpus!(&state, &id);
    let handle = &guard[&id];

    let mut sessions = handle.sessions.lock().await;
    if sessions.remove_session(&sid).is_some() {
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

// ---------------------------------------------------------------------------
// Admin
// ---------------------------------------------------------------------------

async fn daemon_status(State(state): State<AppState>) -> impl IntoResponse {
    Json(DaemonStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: state.uptime_secs(),
        memory_mb: iris_core::mem_profile::rss_mb().unwrap_or(0.0),
        model: state.registry.config().default_model.clone(),
        model_dimension: state.registry.embedder().dimension(),
        corpora: state.registry.list().await,
    })
}
