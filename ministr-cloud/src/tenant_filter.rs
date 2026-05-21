//! Postgres-backed [`TenantCorpusFilter`] for the cloud serve pod.
//!
//! F2.x-b — closes the cross-tenant read leak that F2.x-a (`Backend::Registry`)
//! left open. The MCP `/mcp` surface now consults this filter before
//! dispatching a tool call against a `project = corpus_id`, returning the
//! same shape as a typo (empty results) when the caller does not own the
//! corpus.
//!
//! ## Permissive on `NULL` `tenant_id`
//!
//! Existing rows in `cloud_corpora` from before this change have
//! `tenant_id IS NULL`. Allowing those rows on any tenant lookup
//! preserves backward compatibility while new corpora get their
//! `tenant_id` populated upstream. Once the back-fill is done (separate
//! chunk, F2.x-d), the permissive arm can be tightened.

use std::sync::Arc;

use deadpool_postgres::Pool;
use ministr_api::tenant_filter::{TenantCorpusFilter, TenantFilterError, TenantFilterFuture};

/// Postgres-backed tenant-corpus access decision.
///
/// Cheap to clone — wraps an `Arc<Pool>`.
#[derive(Debug, Clone)]
pub struct PostgresTenantCorpusFilter {
    pool: Arc<Pool>,
}

impl PostgresTenantCorpusFilter {
    #[must_use]
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

fn map_err<E: std::fmt::Display>(prefix: &str) -> impl FnOnce(E) -> TenantFilterError + '_ {
    move |e| TenantFilterError::Storage(format!("{prefix}: {e}"))
}

impl TenantCorpusFilter for PostgresTenantCorpusFilter {
    fn allowed<'a>(
        &'a self,
        tenant_subject: &'a str,
        corpus_id: &'a str,
    ) -> TenantFilterFuture<'a> {
        Box::pin(async move {
            let client = self
                .pool
                .get()
                .await
                .map_err(map_err("tenant filter: get conn"))?;

            // Single-row PK lookup: cheapest possible. Returns the owning
            // tenant_id (may be NULL for legacy rows). Absence of a row
            // = unknown corpus = deny.
            let row = client
                .query_opt(
                    "SELECT tenant_id FROM cloud_corpora WHERE corpus_id = $1",
                    &[&corpus_id],
                )
                .await
                .map_err(map_err("tenant filter: query"))?;

            let Some(row) = row else {
                return Ok(false);
            };
            let owner: Option<String> = row.get("tenant_id");
            match owner {
                None => Ok(true), // legacy / pre-multi-tenant — permissive
                Some(t) => Ok(t == tenant_subject),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impl_is_dyn_compatible() {
        fn assert_dyn(_: &dyn TenantCorpusFilter) {}
        let pool = build_dummy_pool();
        let filter = PostgresTenantCorpusFilter::new(Arc::new(pool));
        assert_dyn(&filter);
    }

    fn build_dummy_pool() -> Pool {
        use deadpool_postgres::{Config, ManagerConfig, RecyclingMethod, Runtime};
        use tokio_postgres::NoTls;
        let mut cfg = Config::new();
        cfg.url = Some("postgres://invalid:invalid@127.0.0.1:1/invalid".to_string());
        cfg.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });
        cfg.create_pool(Some(Runtime::Tokio1), NoTls)
            .expect("create_pool")
    }
}
