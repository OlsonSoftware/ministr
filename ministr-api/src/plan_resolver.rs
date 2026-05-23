//! F5.5-a-plan-lookup — billing-plan resolution seam.
//!
//! Open-core boundary that lets `ministr-mcp`'s OAuth token-validation
//! path reflect the requesting user's real billing tier without
//! depending on `ministr-cloud`. The cloud crate ships a
//! `PostgresPlanResolver` that maps an OAuth subject (the `users.id`
//! UUID minted at sign-in) to `users.plan_id`; self-hosted serve leaves
//! the resolver `None` and the existing `Tenant::local()` default of
//! `Plan::Pro` is preserved.
//!
//! # Why a separate trait — and not just rolling into `OAuthStore`
//!
//! The same reason `ApiKeyResolver` lives here: the trait holds the
//! shape `ministr-mcp` (MIT) needs without forcing the closed-cloud
//! `users` schema into the open-core surface. The Postgres impl lives
//! in `ministr-cloud` and is wired into `OAuthStore` via
//! `with_plan_resolver` at cloud-serve startup. The OAuth path was
//! previously a documented gap (F5.5-a-priority's honest caveat): the
//! resolved `Tenant.plan` always defaulted to `Plan::Pro`, so the
//! `priority=4` Enterprise lane shipped in F5.5-a-priority was
//! structurally unreachable through OAuth even though `queue_priority`
//! and the producer-side stamp were in place.
//!
//! The api-key path was never affected — it carries `plan_id` on
//! `ResolvedApiKey` and parses it inline.

use std::future::Future;
use std::pin::Pin;

/// Errors a [`PlanResolver`] implementation can surface to the
/// `OAuthStore`.
#[derive(Debug, thiserror::Error)]
pub enum PlanResolverError {
    /// Storage layer rejected the lookup (network, schema drift, etc.).
    /// Treated as "no plan known" by the resolver loop — the
    /// `OAuthStore` falls back to `Tenant::local()` (`Plan::Pro`
    /// default) rather than rejecting the request. Logged at warn so
    /// the operator can spot persistent backend trouble.
    #[error("plan resolver storage: {0}")]
    Storage(String),
}

/// Returned future shape for [`PlanResolver::resolve`].
pub type ResolvePlanFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<String>, PlanResolverError>> + Send + 'a>>;

/// Resolve an OAuth subject (a `users.id` UUID) to that user's wire-
/// shape billing plan id (`"pro"` / `"team"` / `"enterprise"`).
///
/// Wired into `OAuthStore` via `with_plan_resolver`; the OAuth
/// `resolve_tenant` path consults the resolver after `validate_token`
/// returns a subject so the constructed `Tenant.plan` reflects the
/// user's real tier rather than the `Tenant::local()` default of Pro.
///
/// Implementations return `Ok(None)` for any "subject doesn't match a
/// known user" case (uuid-cast failure for non-UUID subjects like the
/// self-hosted `ministr-tauri` `client_id`; row missing from `users`) so
/// the `OAuthStore` falls back cleanly to `Tenant::local()`. Reserve
/// `Err(Storage)` for genuine backend failures the operator should see.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn PlanResolver>` inside `OAuthStore`.
pub trait PlanResolver: Send + Sync + std::fmt::Debug {
    /// Resolve `subject` to a wire-shape `plan_id` string if the subject
    /// identifies a known user. Returns `Ok(None)` for any "not a
    /// known user / non-UUID subject" case — distinguishable from a
    /// storage error so the `OAuthStore` can fall back cleanly rather
    /// than reject the request.
    ///
    /// # Errors
    ///
    /// Returns [`PlanResolverError::Storage`] when the backend rejects
    /// the query (network, schema drift, etc.). The `OAuthStore` logs
    /// and falls back to `Tenant::local()` so the request still
    /// succeeds at the Pro default.
    fn resolve<'a>(&'a self, subject: &'a str) -> ResolvePlanFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug, Default)]
    struct StubResolver;

    impl PlanResolver for StubResolver {
        fn resolve<'a>(&'a self, subject: &'a str) -> ResolvePlanFuture<'a> {
            Box::pin(async move {
                if subject == "known-enterprise" {
                    Ok(Some("enterprise".to_string()))
                } else if subject == "boom" {
                    Err(PlanResolverError::Storage("synthetic".into()))
                } else {
                    Ok(None)
                }
            })
        }
    }

    #[test]
    fn trait_is_object_safe() {
        let resolver: Arc<dyn PlanResolver> = Arc::new(StubResolver);
        // Compile-time assertion that the trait can live behind dyn.
        let _ = resolver;
    }

    #[tokio::test]
    async fn stub_round_trip() {
        let r = StubResolver;
        let hit = r.resolve("known-enterprise").await.unwrap();
        assert_eq!(hit.as_deref(), Some("enterprise"));
        let miss = r.resolve("nobody").await.unwrap();
        assert!(miss.is_none());
        let err = r.resolve("boom").await;
        assert!(matches!(err, Err(PlanResolverError::Storage(_))));
    }
}
