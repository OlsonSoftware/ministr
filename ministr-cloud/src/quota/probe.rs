//! `UsageProbe` — the DIP seam quota rules ask "how many X does this
//! tenant own right now?".
//!
//! Two concrete impls: [`RegistryProbe`] counts the daemon's in-memory
//! `CorpusRegistry` (correct on self-hosted serve, where every corpus
//! is in-memory by definition); [`PostgresCorporaProbe`] counts
//! `cloud_corpora` rows filtered by `tenant_id` (the cloud-mode
//! answer, since cloud-mode `register_corpus` writes to
//! `cloud_corpora` via `IndexJobSink` but doesn't populate the
//! in-memory registry until the worker indexes — F-Test-1 finding).

use std::pin::Pin;
use std::sync::Arc;

use deadpool_postgres::Pool;
use ministr_daemon::registry::CorpusRegistry;

/// Errors a probe can surface. Kept narrow + string-based so the
/// trait stays dyn-safe and rules don't transitively pick up the
/// backend's error taxonomy.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("probe backend error: {0}")]
    Backend(String),
}

/// The minimal probe contract every backend implements.
///
/// Methods return `Pin<Box<dyn Future>>` instead of `async fn` so the
/// trait stays `dyn`-safe — the rule list is `Vec<Arc<dyn QuotaRule>>`
/// and each rule may call into the probe, so a stable v-table matters.
pub trait UsageProbe: Send + Sync + std::fmt::Debug {
    /// Count of hosted corpora the tenant currently owns. Today the
    /// daemon-side `CorpusRegistry` is the source of truth and the
    /// only existing impl reports the pod-wide total (no tenant
    /// filtering); a future F3 impl tightens to per-tenant ownership.
    fn corpus_count<'a>(
        &'a self,
        tenant_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<u64, ProbeError>> + Send + 'a>>;
}

/// `UsageProbe` backed by the daemon's in-memory registry.
///
/// **Today's behaviour:** counts every corpus on the pod, regardless of
/// `tenant_id`. The argument is accepted for forward-compatibility —
/// when F3 multi-tenant corpus ownership lands, the count is filtered
/// per tenant without changing this trait surface.
#[derive(Clone)]
pub struct RegistryProbe {
    registry: Arc<CorpusRegistry>,
}

impl std::fmt::Debug for RegistryProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `CorpusRegistry` is `pub` but doesn't derive `Debug` — its
        // internal `HashMap<String, CorpusHandle>` carries shared
        // mutable state we'd rather not stringify. Surface the Arc's
        // strong-count as a cheap proxy for "is this a fresh probe or
        // shared with the rest of the pod".
        f.debug_struct("RegistryProbe")
            .field("registry_strong_count", &Arc::strong_count(&self.registry))
            .finish_non_exhaustive()
    }
}

impl RegistryProbe {
    #[must_use]
    pub fn new(registry: Arc<CorpusRegistry>) -> Self {
        Self { registry }
    }
}

impl UsageProbe for RegistryProbe {
    fn corpus_count<'a>(
        &'a self,
        _tenant_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<u64, ProbeError>> + Send + 'a>> {
        Box::pin(async move {
            let list = self.registry.list().await;
            Ok(u64::try_from(list.len()).unwrap_or(u64::MAX))
        })
    }
}

/// `UsageProbe` backed by the cloud Postgres `cloud_corpora` table.
///
/// Tenant-scoped: `SELECT count(*) FROM cloud_corpora WHERE tenant_id
/// = $1`. The `tenant_id` column is TEXT (migration 0003) so the bind
/// is direct — no `::uuid` cast needed. Cloud-mode `register_corpus`
/// writes here via `IndexJobSink` before the worker has indexed
/// anything, so this probe sees registrations the in-memory
/// `CorpusRegistry` doesn't (the F-Test-1 cloud-registry gap).
///
/// Self-hosted serve continues to wire [`RegistryProbe`]; only
/// `cmd_serve_http` with `MINISTR_PG_URL` set substitutes this impl.
#[derive(Clone)]
pub struct PostgresCorporaProbe {
    pool: Arc<Pool>,
}

impl std::fmt::Debug for PostgresCorporaProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresCorporaProbe")
            .field("pool_strong_count", &Arc::strong_count(&self.pool))
            .finish_non_exhaustive()
    }
}

impl PostgresCorporaProbe {
    #[must_use]
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }

    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self::new(pool)
    }
}

impl UsageProbe for PostgresCorporaProbe {
    fn corpus_count<'a>(
        &'a self,
        tenant_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<u64, ProbeError>> + Send + 'a>> {
        Box::pin(async move {
            let client = self
                .pool
                .get()
                .await
                .map_err(|e| ProbeError::Backend(format!("get conn: {e}")))?;
            let row = client
                .query_one(
                    "SELECT count(*)::bigint AS n FROM cloud_corpora WHERE tenant_id = $1",
                    &[&tenant_id],
                )
                .await
                .map_err(|e| ProbeError::Backend(format!("count query: {e}")))?;
            let n: i64 = row
                .try_get("n")
                .map_err(|e| ProbeError::Backend(format!("read count: {e}")))?;
            Ok(u64::try_from(n).unwrap_or(0))
        })
    }
}

/// In-test probe used by rule + middleware tests to inject a canned
/// count without spinning a daemon. `cfg(test)`-only; never compiled
/// in release builds. Hoisted out of the `tests` module so other
/// modules in the same crate can `use` it (clippy's
/// `items-after-test-module` lint requires the order be types-first).
#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct StubProbe {
    pub(crate) count: u64,
}

#[cfg(test)]
impl UsageProbe for StubProbe {
    fn corpus_count<'a>(
        &'a self,
        _tenant_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<u64, ProbeError>> + Send + 'a>> {
        let count = self.count;
        Box::pin(async move { Ok(count) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_returns_canned_count() {
        let p = StubProbe { count: 7 };
        assert_eq!(p.corpus_count("any").await.unwrap(), 7);
    }

    #[tokio::test]
    async fn trait_is_dyn_compatible() {
        let p: Arc<dyn UsageProbe> = Arc::new(StubProbe { count: 3 });
        assert_eq!(p.corpus_count("t").await.unwrap(), 3);
    }
}
