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
use ministr_api::tenant_filter::{
    DefaultCorpusFuture, TenantCorpusFilter, TenantFilterError, TenantFilterFuture,
};

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
                None => return Ok(true), // legacy / pre-multi-tenant — permissive
                Some(t) if t == tenant_subject => return Ok(true),
                Some(_) => {}
            }
            // F3.2-i — direct ownership didn't match. Check the
            // corpus ACL: an org-grant on this corpus + the
            // tenant_subject's membership in that org admits the
            // call. The lookup is a single index-friendly join (see
            // migration 0005's `idx_cloud_corpus_acl_org` partial
            // unique index + the F1.2 `idx_org_members_user`
            // index).
            let acl_row = client
                .query_opt(
                    "SELECT 1
                     FROM cloud_corpus_acl a
                     JOIN org_members m ON m.org_id = a.org_id
                     WHERE a.corpus_id = $1
                       AND a.org_id IS NOT NULL
                       AND m.user_id = $2::uuid
                     LIMIT 1",
                    &[&corpus_id, &tenant_subject],
                )
                .await
                .map_err(map_err("tenant filter: acl query"))?;
            Ok(acl_row.is_some())
        })
    }

    fn default_corpus_for_tenant<'a>(
        &'a self,
        tenant_subject: &'a str,
    ) -> DefaultCorpusFuture<'a> {
        Box::pin(async move {
            let client = self
                .pool
                .get()
                .await
                .map_err(map_err("default corpus: get conn"))?;
            // Pick the tenant's most-recently-created corpus. Index
            // `idx_cloud_corpora_tenant` (migration 0003) covers this
            // exactly: `(tenant_id, created_at DESC) WHERE tenant_id IS
            // NOT NULL`, so the lookup is a single index probe + read.
            let row = client
                .query_opt(
                    "SELECT corpus_id FROM cloud_corpora \
                     WHERE tenant_id = $1 \
                     ORDER BY created_at DESC \
                     LIMIT 1",
                    &[&tenant_subject],
                )
                .await
                .map_err(map_err("default corpus: query"))?;
            Ok(row.map(|r| r.get::<_, String>("corpus_id")))
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
