//! HTTP daemon on a Unix domain socket.
//!
//! Exposes the iris daemon API via axum, listening on `~/.iris/irisd.sock`.
//! This is how the MCP proxy and CLI communicate with the daemon.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::net::UnixListener;
use tracing::info;

use iris_api::corpus::{ListCorporaResponse, RegisterCorpusRequest, RegisterCorpusResponse};
use iris_api::query::{
    ExtractRequest, ExtractResponse, SurveyRequest, SurveyResponse, SymbolsRequest,
    SymbolsResponse,
};
use iris_api::status::DaemonStatus;
use iris_api::ApiError;

use crate::state::AppState;

/// Build the daemon API router.
pub fn router(state: AppState) -> Router {
    Router::new()
        // Corpus management
        .route("/api/v1/corpora", post(register_corpus).get(list_corpora))
        .route(
            "/api/v1/corpora/{id}",
            get(corpus_status).delete(unregister_corpus),
        )
        // Query endpoints
        .route("/api/v1/corpora/{id}/survey", post(survey))
        .route("/api/v1/corpora/{id}/symbols", post(symbols))
        .route("/api/v1/corpora/{id}/definition/{sym}", get(definition))
        .route("/api/v1/corpora/{id}/references/{sym}", get(references))
        .route("/api/v1/corpora/{id}/read/{section}", get(read_section))
        .route("/api/v1/corpora/{id}/extract", post(extract))
        .route("/api/v1/corpora/{id}/toc", post(toc))
        .route("/api/v1/corpora/{id}/related", post(related))
        .route("/api/v1/corpora/{id}/bridge", post(bridge))
        // Admin
        .route("/api/v1/status", get(daemon_status))
        .with_state(state)
}

/// Start the daemon listener on the Unix domain socket.
pub async fn start(state: AppState) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = iris_api::daemon_socket_path();

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Clean up stale socket from a previous run.
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    info!(path = %socket_path.display(), "daemon listening on UDS");

    let app = router(state);
    axum::serve(listener, app).await?;

    let _ = std::fs::remove_file(&socket_path);
    Ok(())
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn err_response(status: StatusCode, code: &str, msg: impl std::fmt::Display) -> impl IntoResponse {
    (
        status,
        Json(ApiError {
            code: code.to_string(),
            message: msg.to_string(),
        }),
    )
}

// ---------------------------------------------------------------------------
// Corpus management handlers
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
        Err(e) => err_response(StatusCode::BAD_REQUEST, "register_failed", e).into_response(),
    }
}

async fn list_corpora(State(state): State<AppState>) -> impl IntoResponse {
    Json(ListCorporaResponse {
        corpora: state.registry.list().await,
    })
}

async fn corpus_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.registry.list().await.into_iter().find(|c| c.id == id) {
        Some(info) => Json(info).into_response(),
        None => err_response(StatusCode::NOT_FOUND, "not_found", format!("corpus '{id}'"))
            .into_response(),
    }
}

async fn unregister_corpus(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.registry.unregister(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Query handlers
// ---------------------------------------------------------------------------

async fn survey(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SurveyRequest>,
) -> impl IntoResponse {
    let guard = match state.registry.get(&id).await {
        Ok(g) => g,
        Err(e) => return err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    };
    let handle = &guard[&id];

    match handle.service.survey(&req.query, req.top_k.unwrap_or(10)).await {
        Ok(results) => {
            let api_results = results
                .into_iter()
                .map(|r| iris_api::query::SurveyResult {
                    content_id: r.content_id,
                    resolution: r.resolution,
                    score: r.score,
                    text: r.text,
                    heading_path: r.heading_path,
                })
                .collect();
            Json(SurveyResponse { results: api_results }).into_response()
        }
        Err(e) => {
            err_response(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response()
        }
    }
}

async fn symbols(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SymbolsRequest>,
) -> impl IntoResponse {
    let guard = match state.registry.get(&id).await {
        Ok(g) => g,
        Err(e) => return err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    };
    let handle = &guard[&id];

    let filter = iris_core::storage::traits::SymbolFilter {
        name: Some(req.query),
        name_exact: None,
        kind: req.kind,
        visibility: req.visibility,
        module: req.module,
        file_path: None,
    };

    match handle.service.search_symbols(&filter).await {
        Ok(records) => {
            let limit = req.limit.unwrap_or(20);
            let symbols = records
                .into_iter()
                .take(limit)
                .map(|s| iris_api::query::SymbolDefinition {
                    id: s.id.0,
                    name: s.name,
                    kind: s.kind,
                    visibility: s.visibility,
                    signature: s.signature,
                    doc_comment: s.doc_comment,
                    file_path: s.file_path,
                    line_start: s.line_start,
                    line_end: s.line_end,
                    heading_path: s
                        .module_path
                        .split("::")
                        .filter(|p| !p.is_empty())
                        .map(String::from)
                        .collect(),
                    source_context: String::new(),
                })
                .collect();
            Json(SymbolsResponse { symbols }).into_response()
        }
        Err(e) => {
            err_response(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response()
        }
    }
}

async fn definition(
    State(state): State<AppState>,
    Path((id, sym)): Path<(String, String)>,
) -> impl IntoResponse {
    let guard = match state.registry.get(&id).await {
        Ok(g) => g,
        Err(e) => return err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    };
    let handle = &guard[&id];

    match handle.service.get_symbol_definition(&sym).await {
        Ok(def) => Json(iris_api::query::SymbolDefinition {
            id: def.id,
            name: def.name,
            kind: def.kind,
            visibility: def.visibility,
            signature: def.signature,
            doc_comment: def.doc_comment,
            file_path: def.file_path,
            line_start: def.line_start,
            line_end: def.line_end,
            heading_path: def.heading_path,
            source_context: def.source_context,
        })
        .into_response(),
        Err(e) => err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    }
}

async fn references(
    State(state): State<AppState>,
    Path((id, sym)): Path<(String, String)>,
) -> impl IntoResponse {
    let guard = match state.registry.get(&id).await {
        Ok(g) => g,
        Err(e) => return err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    };
    let handle = &guard[&id];

    match handle.service.get_symbol_references(&sym, None).await {
        Ok(refs) => {
            let api_refs = refs
                .into_iter()
                .map(|r| iris_api::query::SymbolReference {
                    from_symbol_id: r.from_symbol_id,
                    from_name: r.from_name,
                    from_file: r.from_file,
                    from_line: r.from_line,
                    to_symbol_id: r.to_symbol_id,
                    to_name: r.to_name,
                    to_file: r.to_file,
                    to_line: r.to_line,
                    ref_kind: r.ref_kind,
                })
                .collect();
            Json(iris_api::query::ReferencesResponse { references: api_refs }).into_response()
        }
        Err(e) => {
            err_response(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response()
        }
    }
}

async fn read_section(
    State(state): State<AppState>,
    Path((id, section)): Path<(String, String)>,
) -> impl IntoResponse {
    let guard = match state.registry.get(&id).await {
        Ok(g) => g,
        Err(e) => return err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    };
    let handle = &guard[&id];

    match handle.service.read_section(&section).await {
        Ok(detail) => Json(iris_api::query::SectionDetail {
            section_id: detail.section_id,
            heading_path: detail.heading_path,
            text: detail.text,
            summary: detail.summary,
            claims_available: detail.claims_available,
        })
        .into_response(),
        Err(e) => err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    }
}

async fn extract(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ExtractRequest>,
) -> impl IntoResponse {
    let guard = match state.registry.get(&id).await {
        Ok(g) => g,
        Err(e) => return err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    };
    let handle = &guard[&id];

    match handle
        .service
        .extract_claims(&req.section_id, req.query.as_deref())
        .await
    {
        Ok(claims) => {
            let api_claims = claims
                .into_iter()
                .map(|c| iris_api::query::ClaimResult {
                    claim_id: c.claim_id,
                    text: c.text,
                    relevance: c.relevance,
                })
                .collect();
            Json(ExtractResponse { claims: api_claims }).into_response()
        }
        Err(e) => {
            err_response(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response()
        }
    }
}

async fn toc(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<iris_api::query::TocRequest>,
) -> impl IntoResponse {
    let guard = match state.registry.get(&id).await {
        Ok(g) => g,
        Err(e) => return err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    };
    let handle = &guard[&id];

    match handle.service.toc(req.document_id.as_deref()).await {
        Ok(entries) => {
            let total = entries.len();
            let offset = req.offset.unwrap_or(0);
            let limit = req.limit.unwrap_or(100);
            let api_entries: Vec<_> = entries
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|e| iris_api::query::TocEntry {
                    id: e.section_id.0,
                    title: e.heading_path.last().cloned().unwrap_or_default(),
                    kind: "section".to_string(),
                    depth: e.depth as usize,
                    children: 0,
                    source_path: Some(e.document_id.0),
                })
                .collect();
            Json(iris_api::query::TocResponse {
                entries: api_entries,
                total,
            })
            .into_response()
        }
        Err(e) => {
            err_response(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response()
        }
    }
}

async fn related(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<iris_api::query::RelatedRequest>,
) -> impl IntoResponse {
    let guard = match state.registry.get(&id).await {
        Ok(g) => g,
        Err(e) => return err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    };
    let handle = &guard[&id];

    let relation_types: Option<Vec<iris_core::types::RelationType>> = if req.relation_types.is_empty()
    {
        None
    } else {
        Some(
            req.relation_types
                .iter()
                .filter_map(|s| iris_core::types::RelationType::parse(s))
                .collect(),
        )
    };

    match handle
        .service
        .related_claims(&req.claim_id, relation_types.as_deref())
        .await
    {
        Ok(claims) => {
            let api_claims = claims
                .into_iter()
                .map(|c| iris_api::query::RelatedClaimResult {
                    claim_id: c.claim_id,
                    text: c.text,
                    relation_type: c.relation_type,
                    source_section: c.source_section,
                    confidence: c.confidence,
                })
                .collect();
            Json(iris_api::query::RelatedResponse {
                claims: api_claims,
            })
            .into_response()
        }
        Err(e) => {
            err_response(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response()
        }
    }
}

async fn bridge(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<iris_api::query::BridgeRequest>,
) -> impl IntoResponse {
    let guard = match state.registry.get(&id).await {
        Ok(g) => g,
        Err(e) => return err_response(StatusCode::NOT_FOUND, "not_found", e).into_response(),
    };
    let handle = &guard[&id];

    match handle
        .service
        .query_bridges(
            req.query.as_deref(),
            req.kind.as_deref(),
            req.source_language.as_deref(),
            None,
        )
        .await
    {
        Ok(links) => {
            let limit = req.limit.unwrap_or(50);
            let api_links: Vec<_> = links
                .into_iter()
                .take(limit)
                .map(|l| iris_api::query::BridgeLink {
                    kind: l.kind,
                    source: l.export_binding_key,
                    source_language: l.export_language,
                    target: l.import_binding_key,
                    target_language: l.import_language,
                    confidence: l.confidence,
                })
                .collect();
            Json(iris_api::query::BridgeResponse { links: api_links }).into_response()
        }
        Err(e) => {
            err_response(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", e).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Admin
// ---------------------------------------------------------------------------

async fn daemon_status(State(state): State<AppState>) -> impl IntoResponse {
    let corpora = state.registry.list().await;
    let rss = iris_core::mem_profile::rss_mb().unwrap_or(0.0);

    Json(DaemonStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: state.uptime_secs(),
        memory_mb: rss,
        model: state.registry.config().default_model.clone(),
        model_dimension: state.registry.embedder().dimension(),
        corpora,
    })
}
