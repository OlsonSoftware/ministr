//! Postgres connection + migration runner for the cloud crate.
//!
//! The F1.2 schema lives under `ministr-cloud/migrations/`. Each
//! migration is a forward-only SQL file embedded via `include_str!` and
//! applied in numeric order; the runner records the latest applied
//! version in `schema_migrations` so re-runs short-circuit.
//!
//! # Pooling + TLS
//!
//! Mirrors `ministr-mcp/src/auth/storage/postgres.rs` — deadpool-postgres
//! for connection pooling, rustls + Mozilla CA bundle for TLS. Azure
//! Postgres Flex requires TLS server-side and there is no opt-out; local
//! integration tests pointed at a plaintext server set
//! `MINISTR_TEST_PG_URL` and accept the same constraint.
//!
//! # What lives here vs. ministr-mcp
//!
//! `ministr-mcp/src/auth/storage/postgres.rs` owns the OAuth tables
//! (`oauth_clients`, `oauth_codes`, `oauth_tokens`). Those schemas are
//! self-contained and idempotent inside the OAuth backend; they pre-date
//! F1.2 and stay where they are. This module owns the F1.2 tenancy
//! schema (`users`, `orgs`, `org_members`, `corpora`, `corpus_acl`,
//! `api_keys`, `usage_events`, `audit_events`) — the tables every
//! cloud-only handler reads through.

use std::str::FromStr;

use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use rustls::ClientConfig;
use tokio_postgres_rustls::MakeRustlsConnect;
use tracing::{debug, info};

/// All forward-only migrations in the cloud crate, ordered by version.
///
/// Append to the end when adding a migration; never reorder, never
/// renumber. The runner applies the smallest unapplied version first.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_initial.sql")),
    (2, include_str!("../migrations/0002_usage_rollups.sql")),
    (3, include_str!("../migrations/0003_corpus_registry.sql")),
    (4, include_str!("../migrations/0004_org_invites.sql")),
    (5, include_str!("../migrations/0005_cloud_corpus_acl.sql")),
    (6, include_str!("../migrations/0006_api_keys_columns.sql")),
    (7, include_str!("../migrations/0007_webhook_subscriptions.sql")),
    (8, include_str!("../migrations/0008_agent_sessions.sql")),
    (9, include_str!("../migrations/0009_session_drops.sql")),
    (10, include_str!("../migrations/0010_org_saml_configs.sql")),
    (11, include_str!("../migrations/0011_org_oidc_configs.sql")),
    (12, include_str!("../migrations/0012_org_oidc_group_role_map.sql")),
    (13, include_str!("../migrations/0013_audit_events_partitioned.sql")),
    (14, include_str!("../migrations/0014_org_siem_configs.sql")),
];

/// Errors surfaced by [`connect`] and [`run_migrations`].
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// Pool construction failed — usually a malformed connection URL.
    #[error("pool: {0}")]
    Pool(String),
    /// Acquiring a connection from the pool failed.
    #[error("get connection: {0}")]
    GetConn(String),
    /// A SQL statement returned an error.
    #[error("sql: {0}")]
    Sql(String),
}

/// Open a deadpool-postgres pool against `url`.
///
/// `url` is a standard libpq connection string
/// (`postgres://user:pw@host/db?sslmode=require`). TLS is wired
/// unconditionally; see the module-level docs.
///
/// # Errors
///
/// Returns [`DbError::Pool`] if `url` is malformed or deadpool refuses
/// to construct a pool.
pub fn connect(url: &str) -> Result<Pool, DbError> {
    let mut cfg = Config::new();
    cfg.url = Some(url.to_string());
    cfg.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    });

    let tls = make_rustls_connector();
    let pool = cfg
        .create_pool(Some(Runtime::Tokio1), tls)
        .map_err(|e| DbError::Pool(format!("create_pool: {e}")))?;

    let host_hint = redact_url_host(url);
    debug!(host = %host_hint, "opened ministr-cloud postgres pool");
    Ok(pool)
}

/// Apply every [`MIGRATIONS`] entry that has not yet been recorded in
/// `schema_migrations`. Idempotent — calling twice in a row is cheap
/// and a no-op after the first successful run.
///
/// Each migration runs inside its own implicit transaction (the SQL
/// files wrap themselves in `BEGIN; ... COMMIT;`); the bookkeeping
/// `INSERT INTO schema_migrations` runs in a separate statement after
/// the migration body succeeds. The runner therefore tolerates a crash
/// between the body and the bookkeeping by relying on the body's
/// own `IF NOT EXISTS` semantics on re-run.
///
/// # Errors
///
/// Returns [`DbError::GetConn`] if a pooled connection can't be
/// acquired, or [`DbError::Sql`] if a migration body fails.
pub async fn run_migrations(pool: &Pool) -> Result<(), DbError> {
    let client = pool
        .get()
        .await
        .map_err(|e| DbError::GetConn(format!("migrations: {e}")))?;

    // Bootstrap the bookkeeping table itself — the first migration
    // also creates it (idempotently) but we need it to exist before we
    // can SELECT from it on a fresh database.
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 version    BIGINT      PRIMARY KEY,
                 applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
             );",
        )
        .await
        .map_err(|e| DbError::Sql(format!("bootstrap schema_migrations: {e}")))?;

    let applied: Vec<i64> = client
        .query("SELECT version FROM schema_migrations", &[])
        .await
        .map_err(|e| DbError::Sql(format!("list applied: {e}")))?
        .iter()
        .map(|row| row.get::<_, i64>("version"))
        .collect();

    for (version, sql) in MIGRATIONS {
        if applied.contains(version) {
            continue;
        }
        info!(version, "applying ministr-cloud migration");
        client
            .batch_execute(sql)
            .await
            .map_err(|e| DbError::Sql(format!("migration {version}: {e}")))?;
        client
            .execute(
                "INSERT INTO schema_migrations (version) VALUES ($1)
                 ON CONFLICT (version) DO NOTHING",
                &[version],
            )
            .await
            .map_err(|e| DbError::Sql(format!("record migration {version}: {e}")))?;
    }

    Ok(())
}

fn make_rustls_connector() -> MakeRustlsConnect {
    // Mozilla CA bundle — same shape as ministr-mcp's OAuth Postgres
    // backend. Sufficient for Azure Postgres Flex, AWS RDS, Google
    // Cloud SQL, and any server with a publicly-trusted chain.
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    MakeRustlsConnect::new(config)
}

fn redact_url_host(url: &str) -> String {
    tokio_postgres::Config::from_str(url)
        .ok()
        .and_then(|cfg| cfg.get_hosts().first().cloned())
        .map_or_else(|| "<unknown>".to_owned(), |h| format!("{h:?}"))
}

#[cfg(test)]
mod tests {
    //! Integration tests. Require a real Postgres at `MINISTR_TEST_PG_URL`
    //! (e.g. `postgres://ministr:ministr@localhost:5432/ministr_test`).
    //! Marked `#[ignore]` so the default `cargo test` run stays
    //! dependency-free; CI flips the env var and reruns with
    //! `cargo test -- --ignored`.

    use super::*;

    fn test_url() -> Option<String> {
        std::env::var("MINISTR_TEST_PG_URL").ok()
    }

    async fn fresh_pool() -> Option<Pool> {
        let url = test_url()?;
        let pool = connect(&url).expect("connect");
        // Drop every F1.2 table so each test starts from a clean slate.
        // Order: leaf tables first, then referenced parents.
        let client = pool.get().await.expect("conn");
        client
            .batch_execute(
                "DROP TABLE IF EXISTS cloud_corpus_acl CASCADE;
                 DROP TABLE IF EXISTS audit_events CASCADE;
                 DROP TABLE IF EXISTS usage_events CASCADE;
                 DROP TABLE IF EXISTS api_keys CASCADE;
                 DROP TABLE IF EXISTS corpus_acl CASCADE;
                 DROP TABLE IF EXISTS corpora CASCADE;
                 DROP TABLE IF EXISTS org_invites CASCADE;
                 DROP TABLE IF EXISTS org_members CASCADE;
                 DROP TABLE IF EXISTS orgs CASCADE;
                 DROP TABLE IF EXISTS users CASCADE;
                 DROP TABLE IF EXISTS schema_migrations CASCADE;",
            )
            .await
            .expect("drop tables");
        Some(pool)
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn migrations_apply_on_fresh_database() {
        let Some(pool) = fresh_pool().await else {
            return;
        };
        run_migrations(&pool).await.expect("migrate");
        let client = pool.get().await.unwrap();
        // Spot-check that every F1.2 table now exists.
        for table in [
            "users",
            "orgs",
            "org_members",
            "corpora",
            "corpus_acl",
            "api_keys",
            "usage_events",
            "audit_events",
            "schema_migrations",
        ] {
            let row = client
                .query_one(
                    "SELECT to_regclass($1::text) IS NOT NULL AS exists",
                    &[&table],
                )
                .await
                .unwrap();
            assert!(row.get::<_, bool>("exists"), "table {table} missing");
        }
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn migrations_are_idempotent() {
        let Some(pool) = fresh_pool().await else {
            return;
        };
        run_migrations(&pool).await.expect("first");
        run_migrations(&pool).await.expect("second");
        let client = pool.get().await.unwrap();
        let row = client
            .query_one("SELECT COUNT(*) AS n FROM schema_migrations", &[])
            .await
            .unwrap();
        // Exactly one row per `MIGRATIONS` entry — no duplicates from
        // the second run.
        let count = row.get::<_, i64>("n");
        let expected = i64::try_from(MIGRATIONS.len()).expect("migrations fit i64");
        assert_eq!(count, expected);
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn corpus_owner_check_rejects_double_owner() {
        let Some(pool) = fresh_pool().await else {
            return;
        };
        run_migrations(&pool).await.unwrap();
        let client = pool.get().await.unwrap();
        client
            .execute(
                "INSERT INTO users (email, plan_id) VALUES ($1, $2)",
                &[&"u@example.com", &"pro"],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO orgs (name, plan_id) VALUES ($1, $2)",
                &[&"acme", &"team"],
            )
            .await
            .unwrap();
        // SQL-side subqueries keep uuid types out of the Rust boundary —
        // the cloud crate doesn't depend on the uuid crate yet.
        let res = client
            .execute(
                "INSERT INTO corpora (owner_user_id, owner_org_id, name)
                 VALUES (
                     (SELECT id FROM users LIMIT 1),
                     (SELECT id FROM orgs  LIMIT 1),
                     'x'
                 )",
                &[],
            )
            .await;
        assert!(res.is_err(), "double-owner insert should fail");
        let res = client
            .execute("INSERT INTO corpora (name) VALUES ('y')", &[])
            .await;
        assert!(res.is_err(), "no-owner insert should fail");
        // Sanity: single-owner insert succeeds.
        client
            .execute(
                "INSERT INTO corpora (owner_user_id, name)
                 VALUES ((SELECT id FROM users LIMIT 1), 'z')",
                &[],
            )
            .await
            .expect("single-owner insert succeeds");
    }
}
