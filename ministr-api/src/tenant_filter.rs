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
pub type DefaultCorpusFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<String>, TenantFilterError>> + Send + 'a>>;

/// Future shape returned by [`TenantCorpusVisibility::visible_corpus_ids`].
/// Yields the set of `corpus_id`s a tenant is allowed to see — `None`
/// means "no filter applied" (self-hosted serve), `Some(vec)` is the
/// explicit allow-list.
pub type VisibleCorpusFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Vec<String>>, TenantFilterError>> + Send + 'a>>;

/// Minimal view of a corpus registration row, shaped for the daemon's
/// `list_corpora` handler to synthesise a `CorpusInfo` when the
/// in-memory `CorpusRegistry` hasn't picked the corpus up yet.
///
/// F-Test-1 finding: cloud-mode `register_corpus` writes to
/// `cloud_corpora` via `IndexJobSink` but never updates the in-memory
/// `CorpusRegistry`, so `GET /api/v1/corpora` returned empty even for
/// the corpus's owner until the worker indexed it. This shape carries
/// just enough to render a pending-status row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusRegistrationView {
    /// `cloud_corpora.corpus_id` (TEXT PK).
    pub id: String,
    /// `cloud_corpora.paths` (deserialised from the JSONB column).
    pub paths: Vec<String>,
    /// `cloud_corpora.display_name` — `None` when the caller didn't
    /// supply one at registration.
    pub display_name: Option<String>,
    /// Raw `cloud_corpora.status` (`"pending"`, `"indexing"`,
    /// `"completed"`, `"failed"`). Daemon synthesis maps this to the
    /// nearest `IndexingStatus` variant; consumers expecting a closed
    /// vocabulary should treat anything outside the known set as
    /// `pending`.
    pub status: String,
}

/// Future shape returned by
/// [`TenantCorpusVisibility::pending_corpora_for_tenant`].
pub type PendingCorporaFuture<'a> = Pin<
    Box<dyn Future<Output = Result<Vec<CorpusRegistrationView>, TenantFilterError>> + Send + 'a>,
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
    fn allowed<'a>(&'a self, tenant_subject: &'a str, corpus_id: &'a str)
    -> TenantFilterFuture<'a>;

    /// F2.x-c — return the `corpus_id` the resolver should answer with
    /// when a tool call comes in with no `project` argument and the
    /// caller's `tenant_subject` is known. Default impl returns
    /// `Ok(None)` so the caller falls back to its existing
    /// `default_service` (the startup-bound placeholder). The Postgres
    /// impl overrides this to return the tenant's most-recently-created
    /// corpus, so authenticated cloud calls without a `project`
    /// argument land on a real corpus instead of the empty default.
    fn default_corpus_for_tenant<'a>(&'a self, tenant_subject: &'a str) -> DefaultCorpusFuture<'a> {
        let _ = tenant_subject;
        Box::pin(async { Ok(None) })
    }
}

/// F3.2-iii — decide which `corpus_id`s a tenant is allowed to see
/// when enumerating corpora (the GET `/api/v1/corpora` list).
///
/// Decoupled from [`TenantCorpusFilter`] because the cardinality is
/// different — list operations return a set, access-control checks
/// return a yes/no. The open-core seam is the same: trait lives in
/// `ministr-api` (MIT) so the daemon's `AppState` can store it
/// without depending on `ministr-cloud`.
///
/// # Semantics
///
/// `None` ⇒ no filter applied. Callers should return the full list
/// (preserves the self-hosted / single-tenant `ministr serve`
/// posture where no visibility filter is wired).
///
/// `Some(vec)` ⇒ exhaustive allow-list. Callers intersect with the
/// in-memory registry's list and return only the survivors.
pub trait TenantCorpusVisibility: Send + Sync + std::fmt::Debug {
    /// Return the set of `corpus_id`s `tenant_subject` is allowed to
    /// see. In the cloud implementation, this is the union of
    /// `cloud_corpora.tenant_id = tenant_subject` and any
    /// `cloud_corpus_acl` grants the tenant inherits through
    /// `org_members`.
    fn visible_corpus_ids<'a>(&'a self, tenant_subject: &'a str) -> VisibleCorpusFuture<'a>;

    /// Return registration rows for corpora the tenant owns directly
    /// (the cloud-side source of truth — `cloud_corpora.tenant_id =
    /// tenant_subject`). Used by the daemon's `list_corpora` handler
    /// to synthesise a `CorpusInfo` for pending corpora that haven't
    /// landed in the in-memory `CorpusRegistry` yet.
    ///
    /// Default impl returns `Vec::new()` so self-hosted serve stays a
    /// pure no-op — the in-memory registry IS the source of truth
    /// there.
    ///
    /// Note this returns only direct-ownership rows, not ACL-grant
    /// rows: ACL-shared corpora must have been indexed before the
    /// grant fires (the granting tenant could see them via the
    /// in-memory registry), so the merged-list path covers them via
    /// the existing `visible_corpus_ids` intersection arm.
    fn pending_corpora_for_tenant<'a>(
        &'a self,
        tenant_subject: &'a str,
    ) -> PendingCorporaFuture<'a> {
        let _ = tenant_subject;
        Box::pin(async { Ok(Vec::new()) })
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

    #[derive(Debug, Default)]
    struct OpenVisibility;

    impl TenantCorpusVisibility for OpenVisibility {
        fn visible_corpus_ids<'a>(&'a self, _: &'a str) -> VisibleCorpusFuture<'a> {
            Box::pin(async { Ok(None) })
        }
    }

    #[test]
    fn trait_is_dyn_compatible() {
        fn assert_dyn(_: &dyn TenantCorpusFilter) {}
        assert_dyn(&AlwaysAllow);
    }

    #[test]
    fn visibility_is_dyn_compatible() {
        fn assert_dyn(_: &dyn TenantCorpusVisibility) {}
        assert_dyn(&OpenVisibility);
    }
}
