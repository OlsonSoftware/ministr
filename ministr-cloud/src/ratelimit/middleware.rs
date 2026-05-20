//! Axum glue. `from_fn_with_state` reads the request, calls
//! [`RateLimitConfig::check`], and either forwards the request to the
//! next layer or short-circuits with 429.
//!
//! Errors are deliberately terse — 429 + `Retry-After` is the wire
//! contract; the body is a small JSON shape clients can render. We
//! never include the bucket key in the error body (avoid leaking
//! whether a given IP / tenant is already on the wall).

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tracing::warn;

use super::bucket::RateLimitDecision;
use super::layer::RateLimitConfig;

/// Axum middleware that applies a single [`RateLimitConfig`]. Stack
/// multiple by layering twice — per-IP first (cheaper extractor),
/// per-tenant after (rejects an authenticated burst).
pub async fn rate_limit_middleware(
    State(cfg): State<Arc<RateLimitConfig>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    match cfg.check(&req) {
        RateLimitDecision::Allowed => next.run(req).await,
        RateLimitDecision::Denied { retry_after } => {
            // u128 → u64 — `as_millis` returns u128 to absorb durations
            // > 584 million years; our retry windows are sub-minute,
            // so saturating to u64::MAX is fine.
            let ms = u64::try_from(retry_after.as_millis()).unwrap_or(u64::MAX);
            warn!(
                limit = %cfg.label,
                retry_after_ms = ms,
                "rate limit hit"
            );
            too_many_requests(cfg.label, retry_after)
        }
    }
}

fn too_many_requests(limit: &'static str, retry_after: Duration) -> Response {
    let body = format!(
        "{{\"error\":\"rate_limited\",\"limit\":\"{limit}\",\"retry_after_ms\":{}}}",
        retry_after.as_millis()
    );
    let mut resp = (StatusCode::TOO_MANY_REQUESTS, body).into_response();
    // RFC 7231 §7.1.3 — `Retry-After` as integer seconds (round up so
    // a fractional 0.4s still tells the client to wait 1s). `as_secs`
    // floors; `as_millis` divided by 1000 with `+1` for any remainder
    // mirrors `ceil` without crossing into floating-point land.
    let secs = retry_after.as_secs() + u64::from(retry_after.subsec_millis() > 0);
    let secs = secs.max(1);
    if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
        resp.headers_mut().insert(header::RETRY_AFTER, v);
    }
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::super::bucket::InMemoryBucket;
    use super::super::key::tenant_key;
    use super::super::layer::RateLimitConfig;
    use super::*;
    use axum::{Router, body::to_bytes, routing::get};
    use ministr_mcp::auth::{Plan, Tenant};
    use std::sync::Arc;
    use tower::ServiceExt as _;

    fn ok_handler() -> Router {
        Router::new().route("/x", get(|| async { "ok" }))
    }

    fn cfg(burst: f64) -> Arc<RateLimitConfig> {
        Arc::new(RateLimitConfig::new(
            Arc::new(InMemoryBucket::new(burst, 0.01)),
            tenant_key::<axum::body::Body>,
            "per-tenant",
        ))
    }

    async fn send(app: Router, with_tenant: bool) -> Response {
        let mut req = Request::builder().uri("/x").body(Body::empty()).unwrap();
        if with_tenant {
            req.extensions_mut().insert(Tenant {
                subject: "t1".into(),
                org_id: None,
                plan: Plan::Pro,
            });
        }
        app.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn first_request_under_burst_is_allowed() {
        let app = ok_handler().layer(axum::middleware::from_fn_with_state(
            cfg(3.0),
            rate_limit_middleware,
        ));
        let resp = send(app, true).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn burst_overflow_returns_429_with_retry_after() {
        let app = ok_handler().layer(axum::middleware::from_fn_with_state(
            cfg(1.0),
            rate_limit_middleware,
        ));
        // First passes…
        let r1 = send(app.clone(), true).await;
        assert_eq!(r1.status(), StatusCode::OK);
        // Second hits the wall (refill is glacial at 0.01/sec).
        let r2 = send(app, true).await;
        assert_eq!(r2.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(r2.headers().contains_key(header::RETRY_AFTER));
        let body = to_bytes(r2.into_body(), 8192).await.unwrap();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("rate_limited"), "body: {body}");
        assert!(body.contains("per-tenant"), "body: {body}");
    }

    #[tokio::test]
    async fn request_without_key_extension_bypasses_limit() {
        let app = ok_handler().layer(axum::middleware::from_fn_with_state(
            cfg(1.0),
            rate_limit_middleware,
        ));
        // No Tenant extension → tenant_key returns None → bucket
        // untouched → unlimited passes.
        for _ in 0..5 {
            let r = send(app.clone(), false).await;
            assert_eq!(r.status(), StatusCode::OK);
        }
    }
}
