//! `UsageProbe` — the DIP seam quota rules ask "how many X does this
//! tenant own right now?".
//!
//! Today's only concrete impl is [`RegistryProbe`], which counts via
//! the daemon's in-memory `CorpusRegistry`. When F3 lands a multi-
//! tenant corpora table the registered count flips to a Postgres
//! query over `corpora.owner_user_id`; the rules never change because
//! they bind to the trait.

use std::pin::Pin;
use std::sync::Arc;

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
