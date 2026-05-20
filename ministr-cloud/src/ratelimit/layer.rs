//! Composition of [`TokenBucket`] + extractor — what `cmd_serve_http`
//! actually passes to axum.
//!
//! [`RateLimitConfig`] holds the two collaborators behind an `Arc` so a
//! single config can be cheaply cloned into every layered route. The
//! middleware in [`super::middleware`] reads through the [`Arc`] on
//! each request — no allocation, no contention beyond the inner
//! bucket's `Mutex`.

use std::sync::Arc;

use super::bucket::TokenBucket;
use axum::http::Request;

/// Function signature every key extractor implements. Plain `fn` (not
/// a trait object) because the small set of extractors is closed and
/// the compiler inlines through the call.
pub type KeyExtractor = for<'a> fn(&'a Request<axum::body::Body>) -> Option<String>;

/// Fully-formed rate-limit configuration. Compose one per route +
/// per-strategy pair (per-IP, per-tenant). The middleware in
/// [`super::middleware`] is a thin shell over this.
///
/// `Clone` is cheap — both fields are `Arc`s / `fn` pointers.
#[derive(Clone)]
pub struct RateLimitConfig {
    bucket: Arc<dyn TokenBucket>,
    extract: KeyExtractor,
    /// Human label for log messages. Set to e.g. `"per-ip"` or
    /// `"per-tenant"` so 429s in the access log say which limit hit.
    pub label: &'static str,
}

impl std::fmt::Debug for RateLimitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimitConfig")
            .field("bucket", &self.bucket)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl RateLimitConfig {
    /// Build a config. `bucket` is the storage; `extract` picks the
    /// key from each request; `label` is what shows up in 429 log lines.
    #[must_use]
    pub fn new(
        bucket: Arc<dyn TokenBucket>,
        extract: KeyExtractor,
        label: &'static str,
    ) -> Self {
        Self {
            bucket,
            extract,
            label,
        }
    }

    /// Apply the rate limit to a request, returning the bucket's
    /// decision. Returns `Allowed` unconditionally when the extractor
    /// returns `None` — handled here (not in the middleware) so the
    /// "no key, no limit" rule lives next to the decision logic.
    pub(crate) fn check(
        &self,
        req: &Request<axum::body::Body>,
    ) -> super::bucket::RateLimitDecision {
        match (self.extract)(req) {
            Some(key) => self.bucket.try_acquire(&key),
            None => super::bucket::RateLimitDecision::Allowed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::bucket::{InMemoryBucket, RateLimitDecision};
    use super::*;
    use axum::body::Body;

    // Adapter helpers that match the `KeyExtractor` signature
    // (`fn(&Request<Body>) -> Option<String>`). The `Option` return is
    // mandated by `KeyExtractor`; `always_same_key` deliberately returns
    // `Some(_)` to exercise the "key present" branch.
    #[allow(clippy::unnecessary_wraps)]
    fn always_same_key(_: &Request<Body>) -> Option<String> {
        Some("k".into())
    }

    fn no_key(_: &Request<Body>) -> Option<String> {
        None
    }

    fn req() -> Request<Body> {
        Request::builder().uri("/").body(Body::empty()).unwrap()
    }

    #[test]
    fn check_consumes_a_token_per_call() {
        let cfg = RateLimitConfig::new(
            Arc::new(InMemoryBucket::new(2.0, 1.0)),
            always_same_key,
            "test",
        );
        assert!(matches!(cfg.check(&req()), RateLimitDecision::Allowed));
        assert!(matches!(cfg.check(&req()), RateLimitDecision::Allowed));
        assert!(matches!(cfg.check(&req()), RateLimitDecision::Denied { .. }));
    }

    #[test]
    fn check_returns_allowed_when_extractor_yields_no_key() {
        let cfg = RateLimitConfig::new(
            Arc::new(InMemoryBucket::new(1.0, 1.0)),
            no_key,
            "test",
        );
        // Many calls in a row, all allowed — extractor returned None
        // every time so the bucket was never touched.
        for _ in 0..10 {
            assert!(matches!(cfg.check(&req()), RateLimitDecision::Allowed));
        }
    }
}
