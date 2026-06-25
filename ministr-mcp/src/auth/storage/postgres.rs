//! Postgres-backed OAuth storage.
//!
//! Mirrors `sqlite.rs` for the cloud (`mcp.ministr.ai`) deployment where
//! multiple pods share a single OAuth state store. Schema and operations
//! are deliberately the same shape — JSON-serialised payload in a `TEXT`
//! column, `expires_at` in a `BIGINT`, atomic single-shot `take_code`.
//!
//! # Pooling
//!
//! deadpool-postgres holds the connection pool. The default config
//! (10 max connections) suits a B1ms Postgres Flex; Enterprise tiers
//! retune via the connection-string `?pool_max_size=N` (we read it
//! here so the operator owns sizing).
//!
//! # TLS
//!
//! tokio-postgres-rustls is wired unconditionally; Azure Postgres Flex
//! requires TLS server-side and there's no opt-out. Trust anchors come
//! from `webpki-roots` (the standard Mozilla CA bundle). Local
//! integration tests that hit a plaintext Postgres do so with their own
//! short-lived self-signed cert; running against an unencrypted server
//! by setting `sslmode=disable` is intentionally not supported by this
//! backend.

use std::str::FromStr;

use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio_postgres::Row;
use tokio_postgres_rustls::MakeRustlsConnect;
use tracing::debug;

use super::super::types::{AccessToken, AuthorizationCode, RegisteredClient};
use super::{OAuthStorage, StorageError, StorageResult};

/// Persistent OAuth storage, deadpool-pooled.
#[derive(Debug, Clone)]
#[allow(dead_code)] // wired into `cmd_serve_http` cloud-mode selector
pub(crate) struct PostgresStorage {
    pool: Pool,
}

#[allow(dead_code)] // wired into `cmd_serve_http` cloud-mode selector
impl PostgresStorage {
    /// Open (or attach to) the OAuth tables in the database referenced by
    /// `url`. Creates the schema idempotently. The URL must be a
    /// standard libpq connection string (`postgres://user:pw@host/db`).
    pub(crate) async fn open(url: &str) -> StorageResult<Self> {
        let mut cfg = Config::new();
        cfg.url = Some(url.to_string());
        cfg.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });

        let tls = make_rustls_connector();
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), tls)
            .map_err(|e| StorageError::Backend(format!("create_pool: {e}")))?;

        let host_hint = redact_url_host(url);
        debug!(host = %host_hint, "opening postgres oauth store");

        ensure_schema(&pool).await?;
        Ok(Self { pool })
    }

    /// Bare-pool constructor for callers that already own a configured
    /// `Pool` (e.g. tests using a custom TLS connector or none at all).
    #[cfg(test)]
    pub(crate) async fn from_pool(pool: Pool) -> StorageResult<Self> {
        ensure_schema(&pool).await?;
        Ok(Self { pool })
    }
}

#[allow(dead_code)] // wired into `cmd_serve_http` cloud-mode selector
fn make_rustls_connector() -> MakeRustlsConnect {
    // Workspace-standard trust policy (Mozilla roots + optional
    // MINISTR_PG_CA_CERT) — see `crate::pg_tls`.
    crate::pg_tls::make_rustls_connector()
}

/// Best-effort host extraction for log messages — never includes the
/// password.
#[allow(dead_code)] // wired into `cmd_serve_http` cloud-mode selector
fn redact_url_host(url: &str) -> String {
    tokio_postgres::Config::from_str(url)
        .ok()
        .and_then(|cfg| cfg.get_hosts().first().cloned())
        .map_or_else(|| "<unknown>".to_owned(), |h| format!("{h:?}"))
}

#[allow(dead_code)] // wired into `cmd_serve_http` cloud-mode selector
async fn ensure_schema(pool: &Pool) -> StorageResult<()> {
    let client = pool
        .get()
        .await
        .map_err(|e| StorageError::Backend(format!("schema get conn: {e}")))?;
    // Same column shapes as the SQLite store (TEXT pk + BIGINT
    // expires_at + TEXT JSON blob) so backend swaps are byte-for-byte.
    // `IF NOT EXISTS` keeps the call idempotent — every pod boots into
    // this path.
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS oauth_clients (
                 client_id TEXT PRIMARY KEY,
                 data      TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS oauth_codes (
                 code       TEXT PRIMARY KEY,
                 expires_at BIGINT NOT NULL,
                 data       TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_oauth_codes_expires
                 ON oauth_codes (expires_at);
             CREATE TABLE IF NOT EXISTS oauth_tokens (
                 token      TEXT PRIMARY KEY,
                 expires_at BIGINT NOT NULL,
                 data       TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_oauth_tokens_expires
                 ON oauth_tokens (expires_at);",
        )
        .await
        .map_err(|e| StorageError::Backend(format!("schema: {e}")))?;
    Ok(())
}

fn row_data(row: &Row) -> StorageResult<String> {
    row.try_get::<_, String>("data")
        .map_err(|e| StorageError::Backend(format!("row.data: {e}")))
}

impl OAuthStorage for PostgresStorage {
    fn save_client(
        &self,
        client: RegisteredClient,
    ) -> impl Future<Output = StorageResult<()>> + Send {
        let pool = self.pool.clone();
        async move {
            let blob = serde_json::to_string(&client)?;
            let conn = pool
                .get()
                .await
                .map_err(|e| StorageError::Backend(format!("save_client conn: {e}")))?;
            conn.execute(
                "INSERT INTO oauth_clients (client_id, data) VALUES ($1, $2)
                 ON CONFLICT (client_id) DO UPDATE SET data = EXCLUDED.data",
                &[&client.client_id, &blob],
            )
            .await
            .map_err(|e| StorageError::Backend(format!("save_client: {e}")))?;
            Ok(())
        }
    }

    fn get_client(
        &self,
        client_id: &str,
    ) -> impl Future<Output = StorageResult<Option<RegisteredClient>>> + Send {
        let pool = self.pool.clone();
        let client_id = client_id.to_owned();
        async move {
            let conn = pool
                .get()
                .await
                .map_err(|e| StorageError::Backend(format!("get_client conn: {e}")))?;
            let row = conn
                .query_opt(
                    "SELECT data FROM oauth_clients WHERE client_id = $1",
                    &[&client_id],
                )
                .await
                .map_err(|e| StorageError::Backend(format!("get_client: {e}")))?;
            match row {
                Some(r) => Ok(Some(serde_json::from_str(&row_data(&r)?)?)),
                None => Ok(None),
            }
        }
    }

    fn save_code(&self, code: AuthorizationCode) -> impl Future<Output = StorageResult<()>> + Send {
        let pool = self.pool.clone();
        async move {
            let blob = serde_json::to_string(&code)?;
            let expires_at = code.expires_at.cast_signed();
            let conn = pool
                .get()
                .await
                .map_err(|e| StorageError::Backend(format!("save_code conn: {e}")))?;
            conn.execute(
                "INSERT INTO oauth_codes (code, expires_at, data) VALUES ($1, $2, $3)
                 ON CONFLICT (code) DO UPDATE
                     SET expires_at = EXCLUDED.expires_at,
                         data       = EXCLUDED.data",
                &[&code.code, &expires_at, &blob],
            )
            .await
            .map_err(|e| StorageError::Backend(format!("save_code: {e}")))?;
            Ok(())
        }
    }

    fn take_code(
        &self,
        code: &str,
    ) -> impl Future<Output = StorageResult<Option<AuthorizationCode>>> + Send {
        let pool = self.pool.clone();
        let code = code.to_owned();
        async move {
            let conn = pool
                .get()
                .await
                .map_err(|e| StorageError::Backend(format!("take_code conn: {e}")))?;
            // `DELETE … RETURNING` is atomic single-statement in Postgres;
            // no explicit transaction needed (and cheaper than the SQLite
            // BEGIN/SELECT/DELETE/COMMIT dance). Concurrent callers see
            // exactly one of them get the row.
            let row = conn
                .query_opt(
                    "DELETE FROM oauth_codes WHERE code = $1 RETURNING data",
                    &[&code],
                )
                .await
                .map_err(|e| StorageError::Backend(format!("take_code: {e}")))?;
            match row {
                Some(r) => Ok(Some(serde_json::from_str(&row_data(&r)?)?)),
                None => Ok(None),
            }
        }
    }

    fn save_token(&self, token: AccessToken) -> impl Future<Output = StorageResult<()>> + Send {
        let pool = self.pool.clone();
        async move {
            let blob = serde_json::to_string(&token)?;
            let expires_at = token.expires_at.cast_signed();
            let conn = pool
                .get()
                .await
                .map_err(|e| StorageError::Backend(format!("save_token conn: {e}")))?;
            conn.execute(
                "INSERT INTO oauth_tokens (token, expires_at, data) VALUES ($1, $2, $3)
                 ON CONFLICT (token) DO UPDATE
                     SET expires_at = EXCLUDED.expires_at,
                         data       = EXCLUDED.data",
                &[&token.token, &expires_at, &blob],
            )
            .await
            .map_err(|e| StorageError::Backend(format!("save_token: {e}")))?;
            Ok(())
        }
    }

    fn get_token(
        &self,
        token: &str,
    ) -> impl Future<Output = StorageResult<Option<AccessToken>>> + Send {
        let pool = self.pool.clone();
        let token = token.to_owned();
        async move {
            let conn = pool
                .get()
                .await
                .map_err(|e| StorageError::Backend(format!("get_token conn: {e}")))?;
            let row = conn
                .query_opt("SELECT data FROM oauth_tokens WHERE token = $1", &[&token])
                .await
                .map_err(|e| StorageError::Backend(format!("get_token: {e}")))?;
            match row {
                Some(r) => Ok(Some(serde_json::from_str(&row_data(&r)?)?)),
                None => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests. Require a real Postgres at `MINISTR_TEST_PG_URL`
    //! (e.g. `postgres://ministr:ministr@localhost:5432/ministr_test`).
    //! Marked `#[ignore]` so the default `cargo test` run stays
    //! dependency-free; CI flips the env var and reruns with
    //! `cargo test -- --ignored`.

    use super::*;
    use crate::auth::util::epoch_now;
    use std::sync::Arc;

    fn test_url() -> Option<String> {
        std::env::var("MINISTR_TEST_PG_URL").ok()
    }

    async fn open() -> Option<PostgresStorage> {
        let url = test_url()?;
        Some(PostgresStorage::open(&url).await.expect("open postgres"))
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn token_round_trip() {
        let Some(storage) = open().await else { return };
        let token = AccessToken {
            token: format!("tok-{}", epoch_now()),
            client_id: "client-a".into(),
            scope: "ministr:read".into(),
            expires_at: epoch_now() + 3600,
        };
        storage.save_token(token.clone()).await.unwrap();
        let got = storage.get_token(&token.token).await.unwrap().unwrap();
        assert_eq!(got.client_id, "client-a");
        assert_eq!(got.scope, "ministr:read");
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn take_code_is_atomic_and_idempotent() {
        let Some(storage) = open().await else { return };
        let code_id = format!("code-{}", epoch_now());
        let code = AuthorizationCode {
            code: code_id.clone(),
            client_id: "client-a".into(),
            redirect_uri: "http://x".into(),
            scope: "ministr:read".into(),
            code_challenge: "abc".into(),
            code_challenge_method: "S256".into(),
            expires_at: epoch_now() + 60,
        };
        storage.save_code(code).await.unwrap();
        assert!(storage.take_code(&code_id).await.unwrap().is_some());
        assert!(storage.take_code(&code_id).await.unwrap().is_none());
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn upsert_replaces_existing_client() {
        let Some(storage) = open().await else { return };
        let cid = format!("c-{}", epoch_now());
        let mut client = RegisteredClient {
            client_id: cid.clone(),
            client_secret: Some("v1".into()),
            redirect_uris: vec!["http://a".into()],
            client_name: None,
            scope: "ministr:read".into(),
            registered_at: epoch_now(),
        };
        storage.save_client(client.clone()).await.unwrap();
        client.client_secret = Some("v2".into());
        storage.save_client(client).await.unwrap();
        let got = storage.get_client(&cid).await.unwrap().unwrap();
        assert_eq!(got.client_secret.as_deref(), Some("v2"));
    }

    /// Sanity-check that the Postgres pool can survive concurrent
    /// `take_code` calls on the same row — exactly one returns Some.
    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn concurrent_take_code_wins_exactly_once() {
        let Some(storage) = open().await else { return };
        let storage = Arc::new(storage);
        let code_id = format!("race-{}", epoch_now());
        storage
            .save_code(AuthorizationCode {
                code: code_id.clone(),
                client_id: "client-a".into(),
                redirect_uri: "http://x".into(),
                scope: "ministr:read".into(),
                code_challenge: "abc".into(),
                code_challenge_method: "S256".into(),
                expires_at: epoch_now() + 60,
            })
            .await
            .unwrap();

        let mut handles = Vec::new();
        for _ in 0..8 {
            let s = storage.clone();
            let id = code_id.clone();
            handles.push(tokio::spawn(async move { s.take_code(&id).await.unwrap() }));
        }
        let mut wins = 0;
        for h in handles {
            if h.await.unwrap().is_some() {
                wins += 1;
            }
        }
        assert_eq!(wins, 1, "exactly one caller wins the race");
    }
}
