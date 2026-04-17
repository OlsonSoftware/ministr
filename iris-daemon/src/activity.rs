//! Activity-recording middleware for the daemon router.
//!
//! Wraps every tool-facing route with a thin axum middleware that
//! derives an [`ActivityEvent`] from the request path + response
//! status and pushes it onto [`AppState::activity`]. Fire-and-forget:
//! if the push fails the tool call is untouched.
//!
//! The middleware keeps the event shape minimal — tool name, corpus id,
//! session id (when present), and wall-clock duration. Handlers that
//! want to enrich the event with request-body detail (e.g. the survey
//! query string) insert an [`ActivitySummary`] into the response
//! extensions; the middleware reads it back before pushing.

use std::time::Instant;

use axum::{
    body::Body,
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use iris_api::activity::ActivityEvent;

use crate::state::AppState;

/// Request-/response-extension handlers can insert to enrich the
/// activity event the middleware records on the way back out.
#[derive(Debug, Clone, Default)]
pub struct ActivitySummary {
    pub summary: Option<String>,
    pub tokens_delta: Option<u64>,
    pub pressure: Option<String>,
    pub cache_hit: bool,
    pub resolution: Option<String>,
}

/// Axum middleware that records a tool-call activity event after a
/// handler runs. Attached globally to the router; non-tool routes
/// (status, corpus CRUD, ingestion progress) are filtered out by path
/// inspection in [`classify_route`].
pub async fn record(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let started = Instant::now();

    let res = next.run(req).await;

    if !res.status().is_success() {
        return res;
    }
    let Some(route) = classify_route(&path) else {
        return res;
    };

    let enrich = res
        .extensions()
        .get::<ActivitySummary>()
        .cloned()
        .unwrap_or_default();

    let event = ActivityEvent {
        timestamp_ms: now_ms(),
        tool: route.tool.to_string(),
        corpus_id: route.corpus_id,
        session_id: route.session_id,
        summary: enrich.summary.unwrap_or(route.path_summary),
        tokens_delta: enrich.tokens_delta,
        pressure: enrich.pressure,
        cache_hit: enrich.cache_hit,
        resolution: enrich.resolution,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    };

    // Push without blocking the response — spawn a detached task so the
    // client receives the body as fast as possible even under buffer
    // contention.
    let state = state.clone();
    tokio::spawn(async move {
        state.push_activity(event).await;
    });

    res
}

struct RouteInfo {
    tool: &'static str,
    corpus_id: String,
    session_id: Option<String>,
    /// Fallback summary derived from path params when the handler didn't
    /// write an [`ActivitySummary`] extension (e.g. section id, symbol id).
    path_summary: String,
}

/// Map a request path to its tool name and key context.
///
/// Returns `None` for non-tool routes (status, corpus CRUD, progress,
/// session-budget, etc.).
fn classify_route(path: &str) -> Option<RouteInfo> {
    let rest = path.strip_prefix("/api/v1/corpora/")?;

    // Split off the corpus id — every tool route has one.
    let (corpus_id, tail) = rest.split_once('/')?;
    let corpus_id = corpus_id.to_string();

    // Session-scoped tool routes: `sessions/{sid}/...`
    if let Some(sess_tail) = tail.strip_prefix("sessions/") {
        let (sid, inner) = sess_tail.split_once('/').unwrap_or((sess_tail, ""));
        let session_id = Some(sid.to_string());
        if let Some(section) = inner.strip_prefix("read/") {
            return Some(RouteInfo {
                tool: "iris_read",
                corpus_id,
                session_id,
                path_summary: section.to_string(),
            });
        }
        if inner == "evicted" {
            return Some(RouteInfo {
                tool: "iris_evicted",
                corpus_id,
                session_id,
                path_summary: String::new(),
            });
        }
        return None;
    }

    let (leaf, arg) = match tail.split_once('/') {
        Some((a, b)) => (a, Some(b.to_string())),
        None => (tail, None),
    };

    let (tool, path_summary) = match leaf {
        "survey" => ("iris_survey", String::new()),
        "symbols" => ("iris_symbols", String::new()),
        "extract" => ("iris_extract", String::new()),
        "toc" => ("iris_toc", String::new()),
        "related" => ("iris_related", String::new()),
        "bridge" => ("iris_bridge", String::new()),
        "compress" => ("iris_compress", String::new()),
        "ask" => ("iris_ask", String::new()),
        "read" => ("iris_read", arg.unwrap_or_default()),
        "definition" => ("iris_definition", arg.unwrap_or_default()),
        "references" => ("iris_references", arg.unwrap_or_default()),
        _ => return None,
    };

    Some(RouteInfo {
        tool,
        corpus_id,
        session_id: None,
        path_summary,
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_survey_route() {
        let r = classify_route("/api/v1/corpora/abc/survey").unwrap();
        assert_eq!(r.tool, "iris_survey");
        assert_eq!(r.corpus_id, "abc");
        assert!(r.session_id.is_none());
    }

    #[test]
    fn classifies_session_read() {
        let r = classify_route("/api/v1/corpora/abc/sessions/sess-1/read/sec-xyz").unwrap();
        assert_eq!(r.tool, "iris_read");
        assert_eq!(r.corpus_id, "abc");
        assert_eq!(r.session_id.as_deref(), Some("sess-1"));
        assert_eq!(r.path_summary, "sec-xyz");
    }

    #[test]
    fn classifies_definition_with_symbol() {
        let r = classify_route("/api/v1/corpora/abc/definition/MySymbol").unwrap();
        assert_eq!(r.tool, "iris_definition");
        assert_eq!(r.path_summary, "MySymbol");
    }

    #[test]
    fn ignores_non_tool_routes() {
        assert!(classify_route("/api/v1/status").is_none());
        assert!(classify_route("/api/v1/corpora/abc").is_none());
        assert!(classify_route("/api/v1/corpora/abc/progress").is_none());
        assert!(classify_route("/api/v1/corpora/abc/sessions/sess-1/budget").is_none());
    }
}
