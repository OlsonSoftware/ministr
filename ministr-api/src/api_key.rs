//! Service-account API key resolution seam.
//!
//! Open-core boundary that lets `ministr-mcp`'s OAuth token-validation
//! path resolve API keys without depending on `ministr-cloud`. The cloud
//! crate ships a `PostgresApiKeyResolver` that hashes the candidate
//! token and looks it up in `api_keys`; self-hosted serve leaves the
//! field `None` and only OAuth tokens authenticate.
//!
//! # Why a separate trait — and not just another OAuth token
//!
//! API keys differ from OAuth tokens in two ways the existing OAuth
//! pipeline can't model cleanly:
//!
//! 1. **Lifecycle**. OAuth tokens have a short TTL; API keys are
//!    long-lived and revoked individually by id (a soft-revoke timestamp
//!    on the row), not by waiting for TTL expiry.
//! 2. **Subject derivation**. OAuth tokens carry a `client_id` that is
//!    itself the subject. API keys are minted *by* a user against
//!    *their own* tenant: the resolver returns the owner's subject
//!    derived from `api_keys.owner_user_id` / `owner_org_id`, plus the
//!    plan resolved through the owner's `users.plan_id` /
//!    `orgs.plan_id`. The middleware then constructs a `Tenant` from
//!    the resolved fields exactly as if the request had carried the
//!    owner's OAuth token.
//!
//! Lives in `ministr-api` (MIT) so `ministr-mcp` (also MIT) can hold an
//! `Option<Arc<dyn ApiKeyResolver>>` without pulling in the closed
//! cloud crate.

use std::future::Future;
use std::pin::Pin;

/// Errors a [`ApiKeyResolver`] implementation can surface to the
/// middleware.
#[derive(Debug, thiserror::Error)]
pub enum ApiKeyError {
    /// Storage layer rejected the lookup (network, schema drift, etc.).
    /// Treated as "not a valid key" by the resolver loop — the
    /// middleware fails closed to a 401.
    #[error("api key storage: {0}")]
    Storage(String),
}

/// One resolved API key, carrying just enough metadata for the
/// middleware to populate `Tenant` exactly as it does for OAuth tokens.
///
/// `plan_id` is a wire-shape string (`"pro" | "team" | "enterprise"`)
/// so this struct stays free of the `Plan` enum (which lives in
/// `ministr-mcp` for handler-facing reasons). The consumer parses it
/// into the enum at the seam.
#[derive(Debug, Clone)]
pub struct ResolvedApiKey {
    /// The key's `api_keys.id` — the resolver returns this so the
    /// middleware can fire-and-forget a `touch_last_used` after the
    /// request completes without re-hashing the token.
    pub key_id: String,
    /// Subject — either `owner_user_id` or `owner_org_id` rendered as a
    /// UUID string. Mirrors the F1.2 tenant model where the subject is
    /// whichever side of the polymorphic ownership is populated.
    pub subject: String,
    /// Org membership. `Some` when the key is owned by an org;
    /// `None` for personal-Pro keys.
    pub org_id: Option<String>,
    /// Resolved billing tier: `"pro"`, `"team"`, or `"enterprise"`.
    /// The middleware maps to its `Plan` enum.
    pub plan_id: String,
    /// Whitespace-separated OAuth-style scopes — matches
    /// `oauth_tokens.scope` shape so the existing scope-check path can
    /// consume them unchanged.
    pub scopes: String,
}

/// Returned future shape for [`ApiKeyResolver::resolve`].
pub type ResolveApiKeyFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<ResolvedApiKey>, ApiKeyError>> + Send + 'a>>;

/// Returned future shape for [`ApiKeyResolver::touch_last_used`].
pub type TouchLastUsedFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), ApiKeyError>> + Send + 'a>>;

/// Resolve raw bearer tokens to API key rows.
///
/// Wired into `OAuthStore` via `with_api_key_resolver`; the OAuth
/// validation path tries OAuth tokens first and only falls through to
/// this resolver when the OAuth lookup misses. That ordering keeps the
/// hot path (OAuth) on its existing single-index probe and pays the
/// extra hash + lookup only for API-key callers.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn ApiKeyResolver>` inside `OAuthStore`.
pub trait ApiKeyResolver: Send + Sync + std::fmt::Debug {
    /// Resolve `raw_token` to a [`ResolvedApiKey`] if the token's hash
    /// matches an active row in `api_keys`. Returns `Ok(None)` for any
    /// "not found / revoked / expired" case — distinguishable from a
    /// storage error so the middleware can fail closed on the latter.
    ///
    /// # Errors
    ///
    /// Returns [`ApiKeyError::Storage`] when the backend rejects the
    /// query (network, schema drift, etc.).
    fn resolve<'a>(&'a self, raw_token: &'a str) -> ResolveApiKeyFuture<'a>;

    /// Fire-and-forget update of `last_used_at` for the given
    /// `api_keys.id`. Called by the middleware *after* a successful
    /// resolve so the touch never blocks the request hot path.
    ///
    /// # Errors
    ///
    /// Returns [`ApiKeyError::Storage`] on write failure. Callers
    /// typically log + drop the error rather than propagate it, since
    /// the request itself already succeeded.
    fn touch_last_used<'a>(&'a self, key_id: &'a str) -> TouchLastUsedFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug)]
    struct StubResolver {
        key_id: &'static str,
    }

    impl ApiKeyResolver for StubResolver {
        fn resolve<'a>(&'a self, _raw_token: &'a str) -> ResolveApiKeyFuture<'a> {
            Box::pin(async move {
                Ok(Some(ResolvedApiKey {
                    key_id: self.key_id.to_string(),
                    subject: "user-uuid".to_string(),
                    org_id: None,
                    plan_id: "pro".to_string(),
                    scopes: "ministr:read".to_string(),
                }))
            })
        }
        fn touch_last_used<'a>(&'a self, _key_id: &'a str) -> TouchLastUsedFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn dyn_dispatch_resolves_through_arc() {
        let r: Arc<dyn ApiKeyResolver> = Arc::new(StubResolver { key_id: "abc" });
        let out = r.resolve("mst_pk_anything").await.unwrap().unwrap();
        assert_eq!(out.key_id, "abc");
        assert_eq!(out.plan_id, "pro");
    }
}
