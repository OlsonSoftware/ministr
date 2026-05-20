//! `QuotaRule` trait + concrete rule impls.
//!
//! Each rule is a single-responsibility check (SRP) that knows its own
//! match predicate and decision logic. The middleware fans the request
//! across the configured rule list; the first [`Decision::Deny`]
//! short-circuits with 402.
//!
//! Adding a new cap (queries/day, indexing minutes) means dropping in
//! a new `impl QuotaRule` next to [`CorpusCountRule`] — the trait, the
//! middleware, and the response builder don't change (OCP).

use std::pin::Pin;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request};
use ministr_mcp::auth::Plan;

use super::caps::caps_for_plan;
use super::probe::{ProbeError, UsageProbe};

/// Outcome of a rule evaluating a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The request is below the cap and may proceed. Subsequent rules
    /// still run.
    Allow,
    /// The rule doesn't apply to this request — skip without touching
    /// the probe. Cheap escape hatch for rules wired into the
    /// middleware but irrelevant to the current method/path.
    Skip,
    /// The request must be rejected with 402. The middleware turns
    /// this into a JSON body the client can render as an upgrade
    /// prompt.
    Deny(Violation),
}

/// Spec-required 402 body fields. Mirrors the §3 paywall payload:
/// `{ reason, upgrade_url }`. Serialised straight into the response
/// body by [`super::middleware`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Violation {
    /// Machine-readable reason tag. Stable wire contract;
    /// MCP clients render an upgrade prompt branched on this value.
    /// Example: `"corpus_quota_exceeded"`.
    pub reason: &'static str,
    /// Absolute URL the user should visit to upgrade. Built from
    /// `https://ministr.ai/billing/upgrade?from=<plan>` so the cloud
    /// landing page lands them on the matching upgrade flow.
    pub upgrade_url: String,
}

/// The contract every rule implements. Dyn-safe via `Pin<Box<Future>>`
/// returns — see [`super::probe::UsageProbe`] for the matching
/// rationale.
pub trait QuotaRule: Send + Sync + std::fmt::Debug {
    /// Returns `true` when this rule is interested in the current
    /// request. `Skip` is the alternative inside `check`, but moving
    /// the predicate up here keeps the cheap path off the probe's
    /// async hot path.
    ///
    /// Concrete `Body` keeps the trait `dyn`-safe (a generic `<B>`
    /// here forbids `Arc<dyn QuotaRule>`); the middleware always
    /// passes `Request<axum::body::Body>` so this is no loss in
    /// practice.
    fn matches(&self, req: &Request<Body>) -> bool;

    /// Evaluate the rule. May call into the probe (async) — that's
    /// why this returns a future rather than running sync.
    fn check<'a>(
        &'a self,
        plan: Plan,
        tenant_id: &'a str,
        probe: &'a Arc<dyn UsageProbe>,
    ) -> Pin<Box<dyn Future<Output = Result<Decision, ProbeError>> + Send + 'a>>;
}

/// Build the canonical upgrade URL for the §3 paywall payload. The
/// `from` query parameter lets `ministr.ai/billing/upgrade` jump
/// straight to the relevant pricing-card highlight.
#[must_use]
pub fn upgrade_url(plan: Plan) -> String {
    let slug = match plan {
        Plan::Pro => "pro",
        Plan::Team => "team",
        Plan::Enterprise => "enterprise",
    };
    format!("https://ministr.ai/billing/upgrade?from={slug}")
}

// ── Concrete rules ────────────────────────────────────────────────────────

/// Enforce the §3 corpus count cap. Fires on `POST /api/v1/corpora`
/// (registering a path) and the clone endpoint
/// `POST /api/v1/corpora/{id}/clone` (clone-and-register, which mints
/// a fresh corpus). Both code paths increment the registry count by
/// one on success — the same cap applies.
///
/// Enterprise has `cap = None` (unlimited); the rule short-circuits
/// to `Allow` without touching the probe.
#[derive(Debug, Clone, Default)]
pub struct CorpusCountRule;

impl QuotaRule for CorpusCountRule {
    fn matches(&self, req: &Request<Body>) -> bool {
        if req.method() != Method::POST {
            return false;
        }
        let path = req.uri().path();
        // POST /api/v1/corpora      — register-by-paths
        // POST /api/v1/corpora/{id}/clone — clone-and-register
        path == "/api/v1/corpora" || path.ends_with("/clone")
    }

    fn check<'a>(
        &'a self,
        plan: Plan,
        tenant_id: &'a str,
        probe: &'a Arc<dyn UsageProbe>,
    ) -> Pin<Box<dyn Future<Output = Result<Decision, ProbeError>> + Send + 'a>> {
        Box::pin(async move {
            let Some(cap) = caps_for_plan(plan).corpora else {
                return Ok(Decision::Allow);
            };
            let current = probe.corpus_count(tenant_id).await?;
            if current >= cap {
                return Ok(Decision::Deny(Violation {
                    reason: "corpus_quota_exceeded",
                    upgrade_url: upgrade_url(plan),
                }));
            }
            Ok(Decision::Allow)
        })
    }
}

/// Atlas access gate (§3) — Pro / Team / Enterprise all admit. Today
/// every authenticated cloud user is on Pro+ (B3: no free cloud), so
/// this is effectively a presence check on `Tenant.plan`. The rule
/// exists so the trait shape is in place; F2.6's Atlas v0 plumbing
/// will start firing it on `GET /atlas/*`.
#[derive(Debug, Clone, Default)]
pub struct AtlasAccessRule;

impl QuotaRule for AtlasAccessRule {
    fn matches(&self, req: &Request<Body>) -> bool {
        req.uri().path().starts_with("/atlas/")
    }

    fn check<'a>(
        &'a self,
        _plan: Plan,
        _tenant_id: &'a str,
        _probe: &'a Arc<dyn UsageProbe>,
    ) -> Pin<Box<dyn Future<Output = Result<Decision, ProbeError>> + Send + 'a>> {
        // Every variant of `Plan` is paid (B3 resolved — no free tier).
        // Future restrictions (e.g. dedicated-Atlas tier) extend `Plan`,
        // not this rule.
        Box::pin(async { Ok(Decision::Allow) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::probe::StubProbe;
    use axum::body::Body;
    use axum::http::Request;

    fn post(path: &str) -> Request<Body> {
        Request::builder()
            .method(Method::POST)
            .uri(path)
            .body(Body::empty())
            .unwrap()
    }

    fn get(path: &str) -> Request<Body> {
        Request::builder()
            .method(Method::GET)
            .uri(path)
            .body(Body::empty())
            .unwrap()
    }

    fn probe(count: u64) -> Arc<dyn UsageProbe> {
        Arc::new(StubProbe { count })
    }

    #[test]
    fn corpus_count_matches_register_and_clone() {
        assert!(CorpusCountRule.matches(&post("/api/v1/corpora")));
        assert!(CorpusCountRule.matches(&post("/api/v1/corpora/abc/clone")));
    }

    #[test]
    fn corpus_count_ignores_other_methods_and_paths() {
        assert!(!CorpusCountRule.matches(&get("/api/v1/corpora")));
        assert!(!CorpusCountRule.matches(&post("/api/v1/corpora/abc")));
        assert!(!CorpusCountRule.matches(&post("/api/v1/healthz")));
    }

    #[tokio::test]
    async fn pro_blocked_at_corpus_cap() {
        let r = CorpusCountRule;
        let p = probe(10);
        match r.check(Plan::Pro, "t", &p).await.unwrap() {
            Decision::Deny(v) => {
                assert_eq!(v.reason, "corpus_quota_exceeded");
                assert_eq!(
                    v.upgrade_url,
                    "https://ministr.ai/billing/upgrade?from=pro"
                );
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pro_allowed_below_corpus_cap() {
        let r = CorpusCountRule;
        let p = probe(9);
        assert_eq!(r.check(Plan::Pro, "t", &p).await.unwrap(), Decision::Allow);
    }

    #[tokio::test]
    async fn team_cap_higher_than_pro() {
        let r = CorpusCountRule;
        let p = probe(10);
        assert_eq!(
            r.check(Plan::Team, "t", &p).await.unwrap(),
            Decision::Allow,
            "team should still pass at corpus #10"
        );
    }

    #[tokio::test]
    async fn enterprise_short_circuits_without_calling_probe() {
        let r = CorpusCountRule;
        let p: Arc<dyn UsageProbe> = Arc::new(StubProbe { count: 1_000_000 });
        assert_eq!(
            r.check(Plan::Enterprise, "t", &p).await.unwrap(),
            Decision::Allow,
            "enterprise is unlimited"
        );
    }

    #[test]
    fn atlas_rule_matches_atlas_paths_only() {
        assert!(AtlasAccessRule.matches(&get("/atlas/react/survey")));
        assert!(!AtlasAccessRule.matches(&get("/api/v1/corpora")));
    }

    #[tokio::test]
    async fn atlas_admits_every_plan() {
        let r = AtlasAccessRule;
        let p = probe(0);
        for plan in [Plan::Pro, Plan::Team, Plan::Enterprise] {
            assert_eq!(r.check(plan, "t", &p).await.unwrap(), Decision::Allow);
        }
    }

    #[test]
    fn upgrade_url_carries_plan_slug() {
        assert_eq!(
            upgrade_url(Plan::Pro),
            "https://ministr.ai/billing/upgrade?from=pro"
        );
        assert_eq!(
            upgrade_url(Plan::Team),
            "https://ministr.ai/billing/upgrade?from=team"
        );
        assert_eq!(
            upgrade_url(Plan::Enterprise),
            "https://ministr.ai/billing/upgrade?from=enterprise"
        );
    }
}
