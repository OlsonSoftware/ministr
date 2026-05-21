//! Per-tenant corpus-access filter.
//!
//! Open-core seam: this trait lives in `ministr-api` (MIT) so cloud-only
//! code (`ministr-cloud`, proprietary) can register an implementation
//! without `ministr-mcp` needing a hard dependency on the cloud crate.
//! Mirrors the [`InstallationTokenMinter`](crate::InstallationTokenMinter)
//! seam introduced in F2.1.
//!
//! ## Semantics
//!
//! [`TenantCorpusFilter::allowed`] returns `true` when the named tenant
//! (identified by its OAuth subject) is permitted to dispatch tool calls
//! against the named corpus. Implementations consult their tenant-mapping
//! store (the cloud `cloud_corpora.tenant_id` column today).
//!
//! When no filter is wired into the MCP backend, the entire
//! [`Backend::Registry`](../../../ministr_mcp/backend/enum.Backend.html#variant.Registry)
//! variant falls back to permissive behaviour — that's the self-hosted /
//! single-tenant `ministr serve` posture. Filters MUST only be wired by
//! cloud mode (`cmd_serve_http` with `MINISTR_PG_URL` set).

use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

/// Errors any [`TenantCorpusFilter`] can surface.
#[derive(Debug, Error)]
pub enum TenantFilterError {
    /// Storage backend failed (Postgres, etc.) — distinct from a `false`
    /// allow decision so callers can log + alert rather than silently
    /// downgrading to the typo-tolerance shape.
    #[error("tenant filter storage error: {0}")]
    Storage(String),
}

/// Future shape returned by [`TenantCorpusFilter::allowed`]. Boxed for
/// `dyn`-safety; mirrors [`crate::corpora_repo::RepoFuture`].
pub type TenantFilterFuture<'a> =
    Pin<Box<dyn Future<Output = Result<bool, TenantFilterError>> + Send + 'a>>;

/// Future shape returned by [`TenantCorpusFilter::default_corpus_for_tenant`].
/// Same `Send + 'a` bounds as [`TenantFilterFuture`], but yielding an
/// `Option<String>` instead of a `bool`.
pub type DefaultCorpusFuture<'a> = Pin<
    Box<dyn Future<Output = Result<Option<String>, TenantFilterError>> + Send + 'a>,
>;

/// Decides whether a tenant may dispatch tool calls against a corpus.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn TenantCorpusFilter>` inside the MCP backend.
pub trait TenantCorpusFilter: Send + Sync + std::fmt::Debug {
    /// Return `Ok(true)` when `tenant_subject` may dispatch against
    /// `corpus_id`. `Ok(false)` is a deny decision and the caller should
    /// fall back to its typo-tolerance shape (empty results, not 403).
    /// `Err` indicates a storage failure — callers should NOT downgrade
    /// to permissive behaviour on Err; treat it as a deny + log.
    fn allowed<'a>(
        &'a self,
        tenant_subject: &'a str,
        corpus_id: &'a str,
    ) -> TenantFilterFuture<'a>;

    /// F2.x-c — return the `corpus_id` the resolver should answer with
    /// when a tool call comes in with no `project` argument and the
    /// caller's `tenant_subject` is known. Default impl returns
    /// `Ok(None)` so the caller falls back to its existing
    /// `default_service` (the startup-bound placeholder). The Postgres
    /// impl overrides this to return the tenant's most-recently-created
    /// corpus, so authenticated cloud calls without a `project`
    /// argument land on a real corpus instead of the empty default.
    fn default_corpus_for_tenant<'a>(
        &'a self,
        tenant_subject: &'a str,
    ) -> DefaultCorpusFuture<'a> {
        let _ = tenant_subject;
        Box::pin(async { Ok(None) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct AlwaysAllow;

    impl TenantCorpusFilter for AlwaysAllow {
        fn allowed<'a>(&'a self, _: &'a str, _: &'a str) -> TenantFilterFuture<'a> {
            Box::pin(async { Ok(true) })
        }
    }

    #[test]
    fn trait_is_dyn_compatible() {
        fn assert_dyn(_: &dyn TenantCorpusFilter) {}
        assert_dyn(&AlwaysAllow);
    }
}
