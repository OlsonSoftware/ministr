//! Axum middleware that fans the configured [`QuotaRule`] list across
//! the incoming request and short-circuits the first `Deny` into HTTP
//! 402 + the §3 paywall JSON body.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ministr_mcp::auth::Tenant;
use tracing::{debug, warn};

use super::probe::UsageProbe;
use super::rule::{Decision, QuotaRule, Violation};

/// Configuration the middleware closes over. Cheap to `Clone` — every
/// field is `Arc`-wrapped.
#[derive(Clone)]
pub struct QuotaState {
    rules: Arc<Vec<Arc<dyn QuotaRule>>>,
    probe: Arc<dyn UsageProbe>,
}

impl std::fmt::Debug for QuotaState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuotaState")
            .field("rule_count", &self.rules.len())
            .field("probe", &self.probe)
            .finish()
    }
}

impl QuotaState {
    /// Build a configured state. The rule list is evaluated in order;
    /// the first `Deny` short-circuits the request.
    #[must_use]
    pub fn new(rules: Vec<Arc<dyn QuotaRule>>, probe: Arc<dyn UsageProbe>) -> Self {
        Self {
            rules: Arc::new(rules),
            probe,
        }
    }
}

/// Axum middleware. Mount with `from_fn_with_state(state, quota_middleware)`.
///
/// Unauthenticated requests (no `Tenant` extension) bypass the layer —
/// they never reach a protected route in production because the auth
/// middleware rejects them earlier. Self-hosted serve mounts the
/// daemon without auth, so leaving the bypass in place keeps that
/// surface unchanged.
pub async fn quota_middleware(
    State(state): State<QuotaState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(tenant) = req.extensions().get::<Tenant>().cloned() else {
        return next.run(req).await;
    };

    for rule in state.rules.iter() {
        if !rule.matches(&req) {
            continue;
        }
        match rule.check(tenant.plan, &tenant.subject, &state.probe).await {
            Ok(Decision::Allow | Decision::Skip) => {}
            Ok(Decision::Deny(violation)) => {
                warn!(
                    reason = %violation.reason,
                    plan = ?tenant.plan,
                    subject = %tenant.subject,
                    "quota violation"
                );
                return paywall_response(&violation);
            }
            Err(e) => {
                // Probe failure is treated as fail-open today. The
                // alternative (closed-fail) would brick the cloud on a
                // transient daemon hiccup; the cost-spiral risk is
                // already covered by F2.2 rate limits. Future hardening
                // can flip this when an SLO demands it.
                warn!(error = %e, "quota probe error; allowing request");
            }
        }
    }
    debug!(subject = %tenant.subject, "quota check passed");
    next.run(req).await
}

fn paywall_response(violation: &Violation) -> Response {
    // Spec from §3: HTTP 402 + JSON `{ reason, upgrade_url }`. The
    // serialisation can't fail — both fields are scalar — but if it
    // ever does we fall back to a static payload so the client always
    // sees a 402 rather than a 5xx.
    let body = serde_json::to_string(violation).unwrap_or_else(|_| {
        "{\"reason\":\"quota_exceeded\",\"upgrade_url\":\"https://ministr.ai/billing/upgrade\"}"
            .to_owned()
    });
    let mut resp = (StatusCode::PAYMENT_REQUIRED, body).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::super::probe::StubProbe;
    use super::super::rule::{AtlasAccessRule, CorpusCountRule};
    use super::*;
    use axum::body::to_bytes;
    use axum::routing::post;
    use axum::Router;
    use ministr_mcp::auth::{Plan, Tenant};
    use tower::ServiceExt as _;

    fn app(state: QuotaState) -> Router {
        Router::new()
            .route("/api/v1/corpora", post(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                state,
                quota_middleware,
            ))
    }

    fn tenant(plan: Plan) -> Tenant {
        Tenant {
            subject: "t1".into(),
            org_id: None,
            plan,
        }
    }

    fn state(count: u64) -> QuotaState {
        QuotaState::new(
            vec![
                Arc::new(CorpusCountRule),
                Arc::new(AtlasAccessRule),
            ],
            Arc::new(StubProbe { count }),
        )
    }

    fn req(plan: Plan) -> Request<Body> {
        let mut r = Request::builder()
            .method(axum::http::Method::POST)
            .uri("/api/v1/corpora")
            .body(Body::empty())
            .unwrap();
        r.extensions_mut().insert(tenant(plan));
        r
    }

    #[tokio::test]
    async fn pro_at_cap_gets_402_with_spec_body() {
        let resp = app(state(10)).oneshot(req(Plan::Pro)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PAYMENT_REQUIRED);
        let body = to_bytes(resp.into_body(), 8192).await.unwrap();
        let body = String::from_utf8_lossy(&body);
        // Spec-mandated fields (ROADMAP §4 F2.3 validation line).
        assert!(body.contains("\"reason\":\"corpus_quota_exceeded\""), "{body}");
        assert!(
            body.contains("\"upgrade_url\":\"https://ministr.ai/billing/upgrade?from=pro\""),
            "{body}"
        );
    }

    #[tokio::test]
    async fn pro_under_cap_passes_through() {
        let resp = app(state(9)).oneshot(req(Plan::Pro)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn team_passes_at_corpus_10_pro_would_have_failed() {
        let resp = app(state(10)).oneshot(req(Plan::Team)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn enterprise_unlimited() {
        // Even at one million corpora, enterprise short-circuits.
        let resp = app(state(1_000_000)).oneshot(req(Plan::Enterprise)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unauthenticated_request_bypasses_quota() {
        // No Tenant extension at all → the middleware falls through.
        let mut r = Request::builder()
            .method(axum::http::Method::POST)
            .uri("/api/v1/corpora")
            .body(Body::empty())
            .unwrap();
        // Deliberately don't insert Tenant.
        let _ = r.extensions_mut(); // no-op to satisfy linter on the binding
        let resp = app(state(10)).oneshot(r).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
