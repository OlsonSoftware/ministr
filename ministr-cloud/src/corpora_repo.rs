//! `CorporaRepo` impl backed by the cloud Postgres pool.
//!
//! `PostgresCorporaRepo` is the cloud-side concrete implementation of
//! the [`ministr_api::CorporaRepo`] trait. The daemon's
//! `CorpusRegistry` consults it on `register` / `unregister` /
//! `update_corpus_paths` (to persist the registration) and on `restore`
//! (to repopulate the in-memory map at boot).
//!
//! Schema lives in `0003_corpus_registry.sql`. Distinct from F1.2's
//! UUID-keyed `corpora` table — that one is shaped for the future
//! ACL/billing/owner story; this one is the working pod-shared registry
//! the daemon reads/writes today. See the migration's header for the
//! merge plan when multi-tenant ACL lands.

use std::sync::Arc;

use deadpool_postgres::Pool;
use ministr_api::corpora_repo::{
    CorporaRepo, CorporaRepoError, CorpusRegistration, RepoFuture,
};
use tracing::warn;

/// Postgres-backed `CorporaRepo` for the cloud serve pod.
///
/// Cheap to clone — wraps an `Arc<Pool>`.
#[derive(Debug, Clone)]
pub struct PostgresCorporaRepo {
    pool: Arc<Pool>,
    tenant_id: Option<String>,
}

impl PostgresCorporaRepo {
    /// Construct a repo backed by `pool`. `tenant_id` is `None` for the
    /// single-tenant cloud pod today; set it once multi-tenant lands so
    /// `list` filters to the current tenant's corpora.
    #[must_use]
    pub fn new(pool: Arc<Pool>, tenant_id: Option<String>) -> Self {
        Self { pool, tenant_id }
    }

    /// F2.x-d — resolve the `tenant_id` for an upsert. Precedence:
    /// (1) the current axum request's tenant via
    /// `ministr_mcp::tenant_scope::current` (set by the `scope_tenant`
    /// middleware on the daemon write router); (2) the repo's own
    /// configured `tenant_id` (legacy single-tenant constructor arg).
    /// Returning `None` leaves the row's `tenant_id` NULL, which the
    /// F2.x-b `TenantCorpusFilter` treats as permissive — keep that
    /// arm only for back-compat with rows written before F2.x-d.
    fn resolve_tenant_id(&self) -> Option<String> {
        ministr_mcp::tenant_scope::current().or_else(|| self.tenant_id.clone())
    }
}

fn map_err<E: std::fmt::Display>(prefix: &str) -> impl FnOnce(E) -> CorporaRepoError + '_ {
    move |e| CorporaRepoError::Storage(format!("{prefix}: {e}"))
}

impl CorporaRepo for PostgresCorporaRepo {
    fn upsert<'a>(&'a self, entry: &'a CorpusRegistration) -> RepoFuture<'a, ()> {
        Box::pin(async move {
            let client = self
                .pool
                .get()
                .await
                .map_err(map_err("cloud_corpora upsert: get conn"))?;

            let paths_json = serde_json::to_value(&entry.paths)
                .map_err(map_err("cloud_corpora upsert: serialize paths"))?;

            // F2.x-d — stamp the request's tenant_id when one is in
            // scope, falling back to the repo's configured tenant. The
            // ON CONFLICT clause preserves any previously-set tenant_id
            // on rows that lose their scope (e.g. a `restore()`-driven
            // re-upsert at boot, where no axum request is in play) by
            // COALESCing: never clobber a populated tenant_id with NULL.
            let tenant_id = self.resolve_tenant_id();
            client
                .execute(
                    "INSERT INTO cloud_corpora \
                       (corpus_id, tenant_id, paths, display_name, updated_at) \
                     VALUES ($1, $2, $3::jsonb, $4, now()) \
                     ON CONFLICT (corpus_id) DO UPDATE SET \
                       tenant_id    = COALESCE(EXCLUDED.tenant_id, cloud_corpora.tenant_id), \
                       paths        = EXCLUDED.paths, \
                       display_name = EXCLUDED.display_name, \
                       updated_at   = now()",
                    &[
                        &entry.corpus_id,
                        &tenant_id,
                        &paths_json,
                        &entry.display_name,
                    ],
                )
                .await
                .map_err(map_err("cloud_corpora upsert: execute"))?;
            Ok(())
        })
    }

    fn remove<'a>(&'a self, corpus_id: &'a str) -> RepoFuture<'a, ()> {
        Box::pin(async move {
            let client = self
                .pool
                .get()
                .await
                .map_err(map_err("cloud_corpora remove: get conn"))?;
            client
                .execute(
                    "DELETE FROM cloud_corpora WHERE corpus_id = $1",
                    &[&corpus_id],
                )
                .await
                .map_err(map_err("cloud_corpora remove: execute"))?;
            Ok(())
        })
    }

    fn list(&self) -> RepoFuture<'_, Vec<CorpusRegistration>> {
        Box::pin(async move {
            let client = self
                .pool
                .get()
                .await
                .map_err(map_err("cloud_corpora list: get conn"))?;

            // tenant_id filter is permissive while we ship single-tenant
            // cloud: when configured, restrict to matching rows; when
            // None, list every row (the current pod's expected scope).
            let rows = if let Some(tenant) = &self.tenant_id {
                client
                    .query(
                        "SELECT corpus_id, paths, display_name \
                         FROM cloud_corpora \
                         WHERE tenant_id = $1 \
                         ORDER BY created_at ASC",
                        &[tenant],
                    )
                    .await
                    .map_err(map_err("cloud_corpora list: query"))?
            } else {
                client
                    .query(
                        "SELECT corpus_id, paths, display_name \
                         FROM cloud_corpora \
                         ORDER BY created_at ASC",
                        &[],
                    )
                    .await
                    .map_err(map_err("cloud_corpora list: query"))?
            };

            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let corpus_id: String = row.get("corpus_id");
                let paths_json: serde_json::Value = row.get("paths");
                let display_name: Option<String> = row.get("display_name");
                let paths: Vec<String> = match serde_json::from_value(paths_json) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(
                            corpus_id = %corpus_id,
                            error = %e,
                            "cloud_corpora row has unparseable paths JSON — skipping"
                        );
                        continue;
                    }
                };
                out.push(CorpusRegistration {
                    corpus_id,
                    paths,
                    display_name,
                });
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F2.x-d — `resolve_tenant_id` precedence: task-local first, then
    /// the configured fallback, then `None`. Exercised outside any
    /// `tokio::task_local!` scope, so the task-local read returns
    /// `None` and the configured fallback wins.
    #[tokio::test]
    async fn resolve_tenant_id_falls_back_to_config_outside_scope() {
        let repo = PostgresCorporaRepo::new(Arc::new(build_dummy_pool()), Some("from-config".into()));
        assert_eq!(repo.resolve_tenant_id().as_deref(), Some("from-config"));
    }

    /// `None` config + no scope → `None` (leaves the row's `tenant_id` NULL).
    #[tokio::test]
    async fn resolve_tenant_id_none_outside_scope_with_none_config() {
        let repo = PostgresCorporaRepo::new(Arc::new(build_dummy_pool()), None);
        assert!(repo.resolve_tenant_id().is_none());
    }

    /// Compile-time proof the impl is `dyn`-safe.
    #[test]
    fn impl_is_dyn_compatible() {
        fn assert_dyn(_: &dyn CorporaRepo) {}
        // Construct only the type — pool isn't exercised in this proof.
        // The reachable path is `PostgresCorporaRepo::new`; we go through
        // it to make sure the public constructor accepts the bounds.
        let pool = build_dummy_pool();
        let repo = PostgresCorporaRepo::new(Arc::new(pool), Some("t1".into()));
        assert_dyn(&repo);
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
